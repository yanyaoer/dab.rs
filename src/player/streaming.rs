use log::{debug, info, warn};
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::error::{DabError, DabResult};

/// Streaming audio source that supports reading while downloading
#[derive(Clone)]
pub struct StreamingAudioSource {
    buffer: Arc<RwLock<ByteBuffer>>,
    download_complete: Arc<AtomicBool>,
    read_position: Arc<AtomicUsize>,
    total_size: Arc<AtomicUsize>,
    adaptive_config: Arc<RwLock<AdaptiveBufferingConfig>>,
    network_stats: Arc<RwLock<NetworkStats>>,
}

/// Adaptive buffering configuration
#[derive(Debug, Clone)]
pub struct AdaptiveBufferingConfig {
    pub min_playback_buffer_ms: u32, // Minimum buffer in milliseconds of audio
    pub target_playback_buffer_ms: u32, // Target buffer size
    pub max_playback_buffer_ms: u32, // Maximum buffer size
    pub rebuffer_threshold_ms: u32,  // When to start rebuffering
    pub estimated_bitrate_kbps: u32, // Estimated audio bitrate
    pub network_speed_kbps: u32,     // Current network speed
    pub last_adjustment: Instant,    // Last time config was adjusted
}

/// Network statistics for adaptive buffering
#[derive(Debug, Clone)]
pub struct NetworkStats {
    pub download_speed_kbps: u32,
    pub recent_stalls: u32,
    pub total_bytes_downloaded: u64,
    pub download_start_time: Option<Instant>,
    pub last_speed_measurement: Instant,
}

impl Default for NetworkStats {
    fn default() -> Self {
        Self {
            download_speed_kbps: 0,
            recent_stalls: 0,
            total_bytes_downloaded: 0,
            download_start_time: None,
            last_speed_measurement: Instant::now(),
        }
    }
}

/// Thread-safe byte buffer for streaming audio data
pub struct ByteBuffer {
    data: Vec<u8>,
    write_position: usize,
    read_position: usize, // Track read position within buffer
    min_buffer_size: usize,
    max_buffer_size: usize,
    is_circular: bool, // Enable circular buffer mode for large files
}

impl StreamingAudioSource {
    /// Create a new streaming audio source with adaptive buffering
    pub fn new(min_buffer_mb: u32, max_buffer_mb: u32) -> Self {
        let min_buffer_size = (min_buffer_mb * 1024 * 1024) as usize;
        let max_buffer_size = (max_buffer_mb * 1024 * 1024) as usize;

        let adaptive_config = AdaptiveBufferingConfig {
            min_playback_buffer_ms: 2000,    // 2 seconds minimum
            target_playback_buffer_ms: 5000, // 5 seconds target
            max_playback_buffer_ms: 15000,   // 15 seconds maximum
            rebuffer_threshold_ms: 1000,     // 1 second rebuffer threshold
            estimated_bitrate_kbps: 320,     // Default 320kbps estimate
            network_speed_kbps: 1000,        // Default 1MB/s estimate
            last_adjustment: Instant::now(),
        };

        Self {
            buffer: Arc::new(RwLock::new(ByteBuffer::new(
                min_buffer_size,
                max_buffer_size,
            ))),
            download_complete: Arc::new(AtomicBool::new(false)),
            read_position: Arc::new(AtomicUsize::new(0)),
            total_size: Arc::new(AtomicUsize::new(0)),
            adaptive_config: Arc::new(RwLock::new(adaptive_config)),
            network_stats: Arc::new(RwLock::new(NetworkStats::default())),
        }
    }

    /// Create with custom adaptive configuration
    pub fn new_with_config(
        min_buffer_mb: u32,
        max_buffer_mb: u32,
        config: AdaptiveBufferingConfig,
    ) -> Self {
        let min_buffer_size = (min_buffer_mb * 1024 * 1024) as usize;
        let max_buffer_size = (max_buffer_mb * 1024 * 1024) as usize;

        Self {
            buffer: Arc::new(RwLock::new(ByteBuffer::new(
                min_buffer_size,
                max_buffer_size,
            ))),
            download_complete: Arc::new(AtomicBool::new(false)),
            read_position: Arc::new(AtomicUsize::new(0)),
            total_size: Arc::new(AtomicUsize::new(0)),
            adaptive_config: Arc::new(RwLock::new(config)),
            network_stats: Arc::new(RwLock::new(NetworkStats::default())),
        }
    }

    /// Write data to the buffer (called by download task)
    pub async fn write_data(&self, data: &[u8]) -> DabResult<()> {
        // Update network statistics
        self.update_network_stats(data.len()).await;

        let mut buffer = self.buffer.write().await;
        buffer.write_data(data)?;

        // Check if we should adjust buffering strategy
        if self.should_adjust_buffering().await {
            self.adjust_buffering_strategy().await;
        }

        Ok(())
    }

    /// Mark download as complete
    pub fn mark_complete(&self, total_size: usize) {
        self.total_size.store(total_size, Ordering::Relaxed);
        self.download_complete.store(true, Ordering::Relaxed);
        info!("Download marked complete, total size: {} bytes", total_size);
    }

    /// Check if there's enough data to start playback (adaptive)
    pub async fn is_ready_for_playback(&self) -> bool {
        let buffer = self.buffer.read().await;
        let current_position = self.read_position.load(Ordering::Relaxed);
        let config = self.adaptive_config.read().await;

        // Calculate required bytes based on target buffer time and estimated bitrate
        let target_bytes = self
            .calculate_buffer_bytes_for_duration(config.target_playback_buffer_ms)
            .await;
        let available_bytes = buffer
            .get_available_bytes()
            .saturating_sub(current_position);

        // For streaming format detection, we need a minimum amount of data regardless of calculated buffer
        // Most audio formats can be detected within the first 64KB
        let min_detection_bytes = 65536; // 64KB for format detection
        let required_bytes = target_bytes.max(min_detection_bytes);

        let is_ready = available_bytes >= required_bytes;

        if is_ready {
            debug!(
                "Stream ready: {} bytes available, {} required (target: {}ms, detection: {} bytes)",
                available_bytes,
                required_bytes,
                config.target_playback_buffer_ms,
                min_detection_bytes
            );
        } else {
            debug!(
                "Stream not ready: {} bytes available, {} required (need {} more)",
                available_bytes,
                required_bytes,
                required_bytes.saturating_sub(available_bytes)
            );
        }

        is_ready
    }

    /// Check if we're in a rebuffering state
    pub async fn should_rebuffer(&self) -> bool {
        let buffer = self.buffer.read().await;
        let current_position = self.read_position.load(Ordering::Relaxed);
        let config = self.adaptive_config.read().await;

        let required_bytes = self
            .calculate_buffer_bytes_for_duration(config.rebuffer_threshold_ms)
            .await;
        let available_bytes = buffer
            .get_available_bytes()
            .saturating_sub(current_position);

        available_bytes < required_bytes
    }

    /// Get download progress (0.0 to 1.0)
    pub async fn get_progress(&self) -> f32 {
        let total = self.total_size.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }

        let buffer = self.buffer.read().await;
        let downloaded = buffer.get_available_bytes();
        downloaded as f32 / total as f32
    }

    /// Check if download is complete
    pub fn is_download_complete(&self) -> bool {
        self.download_complete.load(Ordering::Relaxed)
    }

    /// Get available bytes for reading
    pub async fn get_available_bytes(&self) -> usize {
        let buffer = self.buffer.read().await;
        let current_position = self.read_position.load(Ordering::Relaxed);
        buffer
            .get_available_bytes()
            .saturating_sub(current_position)
    }

    /// Get available playback time in milliseconds
    pub async fn get_available_playback_ms(&self) -> u32 {
        let available_bytes = self.get_available_bytes().await;
        let config = self.adaptive_config.read().await;

        if config.estimated_bitrate_kbps == 0 {
            return 0;
        }

        // Convert bytes to milliseconds of audio
        // bitrate is in kbps, so bytes_per_ms = bitrate_kbps * 1000 / 8 / 1000 = bitrate_kbps / 8
        let bytes_per_ms = config.estimated_bitrate_kbps as f32 / 8.0;
        (available_bytes as f32 / bytes_per_ms) as u32
    }

    /// Wait for more data to become available (adaptive timeout)
    pub async fn wait_for_data(&self, timeout_ms: u64) -> bool {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);

        while start.elapsed() < timeout {
            if self.get_available_bytes().await > 0 || self.is_download_complete() {
                return true;
            }

            // Use adaptive sleep interval based on network speed
            let config = self.adaptive_config.read().await;
            let sleep_ms = if config.network_speed_kbps > 1000 {
                5 // Fast network, check more frequently
            } else if config.network_speed_kbps > 500 {
                10 // Medium network
            } else {
                20 // Slow network, check less frequently
            };

            tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
        }
        false
    }
}

impl Read for StreamingAudioSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // For sync Read trait implementation, we need to handle this carefully
        // In an async context, this should not be called directly
        let current_position = self.read_position.load(Ordering::Relaxed);

        // Try to read synchronously from the buffer without blocking
        self.sync_read_fallback(buf, current_position)
    }
}

impl StreamingAudioSource {
    fn sync_read_fallback(
        &self,
        buf: &mut [u8],
        current_position: usize,
    ) -> std::io::Result<usize> {
        // Improved fallback for sync reads with better error handling
        let buffer_guard = match self.buffer.try_read() {
            Ok(guard) => guard,
            Err(_) => {
                // If buffer is locked, wait a short time and try again
                std::thread::sleep(std::time::Duration::from_millis(1));
                match self.buffer.try_read() {
                    Ok(guard) => guard,
                    Err(_) => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::WouldBlock,
                            "Buffer temporarily unavailable",
                        ))
                    }
                }
            }
        };

        // Check if we have data available at the current position
        let (buffer_start, buffer_size, _) = buffer_guard.get_buffer_info();

        if buffer_guard.is_circular {
            // For circular buffer, check if position is within our window
            if current_position < buffer_start {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "Position is behind current buffer window",
                ));
            }

            let buffer_position = current_position - buffer_start;
            if buffer_position >= buffer_size {
                if self.is_download_complete() {
                    return Ok(0); // EOF
                } else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::WouldBlock,
                        "Data not available yet",
                    ));
                }
            }
        } else {
            // For linear buffer
            if current_position >= buffer_size {
                if self.is_download_complete() {
                    return Ok(0); // EOF
                } else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::WouldBlock,
                        "Data not available yet",
                    ));
                }
            }
        }

        let bytes_read = buffer_guard.read_at_position(current_position, buf)?;
        self.read_position.fetch_add(bytes_read, Ordering::Relaxed);
        Ok(bytes_read)
    }
    async fn async_read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        let current_position = self.read_position.load(Ordering::Relaxed);

        // Wait for data if not available and download not complete
        if self.get_available_bytes().await == 0 && !self.is_download_complete() {
            debug!("Waiting for more data at position {}", current_position);
            if !self.wait_for_data(5000).await {
                warn!("Timeout waiting for data");
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Timeout waiting for streaming data",
                ));
            }
        }

        // Read from buffer
        let buffer = self.buffer.read().await;
        let bytes_read = buffer.read_at_position(current_position, buf)?;

        // Update read position
        self.read_position.fetch_add(bytes_read, Ordering::Relaxed);

        debug!("Read {} bytes at position {}", bytes_read, current_position);
        Ok(bytes_read)
    }

    /// Update network statistics based on downloaded data
    async fn update_network_stats(&self, bytes_downloaded: usize) {
        let mut stats = self.network_stats.write().await;
        stats.total_bytes_downloaded += bytes_downloaded as u64;

        // Initialize start time if not set
        if stats.download_start_time.is_none() {
            stats.download_start_time = Some(Instant::now());
            stats.last_speed_measurement = Instant::now();
            return;
        }

        let now = Instant::now();
        let since_last_measurement = now.duration_since(stats.last_speed_measurement);

        // Update speed measurement every second
        if since_last_measurement >= Duration::from_secs(1) {
            let total_duration = now.duration_since(stats.download_start_time.unwrap());
            if total_duration.as_secs() > 0 {
                let speed_kbps = (stats.total_bytes_downloaded / total_duration.as_secs()) / 1024;
                stats.download_speed_kbps = speed_kbps as u32;
                stats.last_speed_measurement = now;

                // Update adaptive config with new speed
                let mut config = self.adaptive_config.write().await;
                config.network_speed_kbps = stats.download_speed_kbps;

                debug!("Network speed updated: {} KB/s", stats.download_speed_kbps);
            }
        }
    }

    /// Check if buffering strategy should be adjusted
    async fn should_adjust_buffering(&self) -> bool {
        let config = self.adaptive_config.read().await;
        let now = Instant::now();

        // Adjust at most once every 5 seconds
        now.duration_since(config.last_adjustment) >= Duration::from_secs(5)
    }

    /// Adjust buffering strategy based on current conditions
    async fn adjust_buffering_strategy(&self) {
        let mut config = self.adaptive_config.write().await;
        let stats = self.network_stats.read().await;

        // Adjust buffer targets based on network conditions
        if stats.download_speed_kbps > 0 {
            let speed_ratio =
                stats.download_speed_kbps as f32 / config.estimated_bitrate_kbps as f32;

            if speed_ratio > 10.0 {
                // Very fast network - reduce buffering
                config.target_playback_buffer_ms =
                    config.target_playback_buffer_ms.max(3000).min(8000);
            } else if speed_ratio > 5.0 {
                // Fast network - moderate buffering
                config.target_playback_buffer_ms =
                    config.target_playback_buffer_ms.max(5000).min(10000);
            } else if speed_ratio > 2.0 {
                // Adequate network - standard buffering
                config.target_playback_buffer_ms =
                    config.target_playback_buffer_ms.max(8000).min(12000);
            } else {
                // Slow network - increase buffering
                config.target_playback_buffer_ms =
                    config.target_playback_buffer_ms.max(10000).min(20000);
                config.rebuffer_threshold_ms = config.rebuffer_threshold_ms.max(2000).min(5000);
            }

            // Handle rebuffering events
            if stats.recent_stalls > 0 {
                config.target_playback_buffer_ms =
                    (config.target_playback_buffer_ms * 15 / 10).min(config.max_playback_buffer_ms);
                config.rebuffer_threshold_ms = (config.rebuffer_threshold_ms * 12 / 10)
                    .min(config.target_playback_buffer_ms / 2);
                info!(
                    "Adjusted buffering due to {} recent stalls: target={}ms, rebuffer={}ms",
                    stats.recent_stalls,
                    config.target_playback_buffer_ms,
                    config.rebuffer_threshold_ms
                );
            }
        }

        config.last_adjustment = Instant::now();
    }

    /// Calculate buffer size in bytes for a given duration in milliseconds
    pub async fn calculate_buffer_bytes_for_duration(&self, duration_ms: u32) -> usize {
        let config = self.adaptive_config.read().await;

        if config.estimated_bitrate_kbps == 0 {
            return 1024 * 1024; // 1MB fallback
        }

        // bitrate_kbps * duration_ms / 8 / 1000 = bytes
        let bytes = (config.estimated_bitrate_kbps as u64 * duration_ms as u64) / 8000;
        bytes as usize
    }

    /// Set estimated bitrate (called when audio format is detected)
    pub async fn set_estimated_bitrate(&self, bitrate_kbps: u32) {
        let mut config = self.adaptive_config.write().await;
        config.estimated_bitrate_kbps = bitrate_kbps;
        info!("Audio bitrate estimated: {} kbps", bitrate_kbps);
    }

    /// Record a stall event for adaptive adjustment
    pub async fn record_stall(&self) {
        let mut stats = self.network_stats.write().await;
        stats.recent_stalls += 1;
        warn!("Playback stall recorded (total: {})", stats.recent_stalls);

        // Reset stall counter periodically
        if stats.recent_stalls > 10 {
            stats.recent_stalls = 5; // Keep some history but prevent overflow
        }
    }

    /// Get current adaptive configuration
    pub async fn get_adaptive_config(&self) -> AdaptiveBufferingConfig {
        self.adaptive_config.read().await.clone()
    }

    /// Get current network statistics
    pub async fn get_network_stats(&self) -> NetworkStats {
        self.network_stats.read().await.clone()
    }

    /// Implement buffer recovery strategy when buffer is insufficient
    pub async fn recover_buffer(&self) -> BufferRecoveryAction {
        let config = self.adaptive_config.read().await;
        let stats = self.network_stats.read().await;
        let available_ms = self.get_available_playback_ms().await;

        // Determine recovery action based on current conditions
        if available_ms < config.rebuffer_threshold_ms {
            if stats.download_speed_kbps < 100 {
                // Very slow network - emergency measures
                BufferRecoveryAction::EmergencyMode {
                    reduce_quality: true,
                    pause_preloading: true,
                    increase_buffer_target: true,
                }
            } else if stats.download_speed_kbps < 500 {
                // Slow network - conservative recovery
                BufferRecoveryAction::ConservativeMode {
                    pause_preloading: true,
                    increase_buffer_target: false,
                }
            } else if stats.recent_stalls > 2 {
                // Good network but frequent stalls - adaptive recovery
                BufferRecoveryAction::AdaptiveMode {
                    adjust_thresholds: true,
                    temporary_buffer_increase: true,
                }
            } else {
                // Temporary hiccup - minimal intervention
                BufferRecoveryAction::MinimalMode {
                    wait_duration_ms: 2000,
                }
            }
        } else {
            BufferRecoveryAction::NoAction
        }
    }

    /// Execute buffer recovery action
    pub async fn execute_recovery_action(&self, action: BufferRecoveryAction) -> DabResult<()> {
        match action {
            BufferRecoveryAction::EmergencyMode {
                reduce_quality,
                pause_preloading,
                increase_buffer_target,
            } => {
                info!("Executing emergency buffer recovery");

                if increase_buffer_target {
                    let mut config = self.adaptive_config.write().await;
                    config.target_playback_buffer_ms =
                        (config.target_playback_buffer_ms * 2).min(config.max_playback_buffer_ms);
                    config.rebuffer_threshold_ms = (config.rebuffer_threshold_ms * 15 / 10)
                        .min(config.target_playback_buffer_ms / 3);
                }

                // Additional emergency measures would be implemented here
                // (quality reduction and preloading pause would need coordination with other components)
                info!("Emergency recovery: quality_reduction={}, pause_preload={}, increase_buffer={}", 
                    reduce_quality, pause_preloading, increase_buffer_target);
            }

            BufferRecoveryAction::ConservativeMode {
                pause_preloading,
                increase_buffer_target,
            } => {
                info!("Executing conservative buffer recovery");

                if increase_buffer_target {
                    let mut config = self.adaptive_config.write().await;
                    config.target_playback_buffer_ms = (config.target_playback_buffer_ms * 13 / 10)
                        .min(config.max_playback_buffer_ms);
                }

                info!(
                    "Conservative recovery: pause_preload={}, increase_buffer={}",
                    pause_preloading, increase_buffer_target
                );
            }

            BufferRecoveryAction::AdaptiveMode {
                adjust_thresholds,
                temporary_buffer_increase,
            } => {
                info!("Executing adaptive buffer recovery");

                let mut config = self.adaptive_config.write().await;

                if adjust_thresholds {
                    // Increase thresholds to be more conservative
                    config.rebuffer_threshold_ms = (config.rebuffer_threshold_ms * 12 / 10)
                        .min(config.target_playback_buffer_ms / 2);
                }

                if temporary_buffer_increase {
                    config.target_playback_buffer_ms = (config.target_playback_buffer_ms * 12 / 10)
                        .min(config.max_playback_buffer_ms);
                }

                info!(
                    "Adaptive recovery: adjust_thresholds={}, temp_increase={}",
                    adjust_thresholds, temporary_buffer_increase
                );
            }

            BufferRecoveryAction::MinimalMode { wait_duration_ms } => {
                info!(
                    "Executing minimal buffer recovery (wait {}ms)",
                    wait_duration_ms
                );

                // Brief pause to allow buffer to recover
                tokio::time::sleep(Duration::from_millis(wait_duration_ms as u64)).await;
            }

            BufferRecoveryAction::NoAction => {
                debug!("No buffer recovery action needed");
            }
        }

        Ok(())
    }

    /// Check if buffer recovery is needed and execute if necessary
    pub async fn check_and_recover_buffer(&self) -> DabResult<bool> {
        let action = self.recover_buffer().await;

        match action {
            BufferRecoveryAction::NoAction => Ok(false),
            _ => {
                self.execute_recovery_action(action).await?;
                Ok(true)
            }
        }
    }
}

impl Seek for StreamingAudioSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        // For sync Seek trait implementation, we need to handle this without async
        let new_position = match pos {
            SeekFrom::Start(offset) => offset as usize,
            SeekFrom::End(offset) => {
                let total_size = self.total_size.load(Ordering::Relaxed);
                if total_size == 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Cannot seek from end: total size unknown",
                    ));
                }
                (total_size as i64 + offset) as usize
            }
            SeekFrom::Current(offset) => {
                let current = self.read_position.load(Ordering::Relaxed);
                (current as i64 + offset) as usize
            }
        };

        // Try to get available bytes synchronously
        let available_bytes = match self.buffer.try_read() {
            Ok(buffer) => buffer.get_available_bytes(),
            Err(_) => {
                // If we can't get the lock, assume no data available
                0
            }
        };

        // Check if seeking beyond available data
        if new_position > available_bytes && !self.is_download_complete() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Cannot seek beyond available data while streaming",
            ));
        }

        self.read_position.store(new_position, Ordering::Relaxed);
        debug!("Seeked to position {}", new_position);
        Ok(new_position as u64)
    }
}

impl Default for AdaptiveBufferingConfig {
    fn default() -> Self {
        Self {
            min_playback_buffer_ms: 2000,
            target_playback_buffer_ms: 5000,
            max_playback_buffer_ms: 15000,
            rebuffer_threshold_ms: 1000,
            estimated_bitrate_kbps: 320,
            network_speed_kbps: 1000,
            last_adjustment: Instant::now(),
        }
    }
}

impl ByteBuffer {
    fn new(min_buffer_size: usize, max_buffer_size: usize) -> Self {
        Self {
            data: Vec::with_capacity(min_buffer_size),
            write_position: 0,
            read_position: 0,
            min_buffer_size,
            max_buffer_size,
            is_circular: true, // Always enable circular buffer for streaming
        }
    }

    fn write_data(&mut self, data: &[u8]) -> DabResult<()> {
        if self.is_circular {
            self.write_data_circular(data)
        } else {
            self.write_data_linear(data)
        }
    }

    fn write_data_linear(&mut self, data: &[u8]) -> DabResult<()> {
        // Check if adding this data would exceed max buffer size
        if self.data.len() + data.len() > self.max_buffer_size {
            return Err(DabError::Io(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "Buffer overflow: max buffer size exceeded",
            )));
        }

        self.data.extend_from_slice(data);
        self.write_position += data.len();
        debug!(
            "Written {} bytes to buffer, total: {}",
            data.len(),
            self.data.len()
        );
        Ok(())
    }

    fn write_data_circular(&mut self, data: &[u8]) -> DabResult<()> {
        // For circular buffer, we maintain a sliding window
        let incoming_len = data.len();

        // If buffer is full, make room by removing old data from the beginning
        if self.data.len() + incoming_len > self.max_buffer_size {
            let bytes_to_remove = (self.data.len() + incoming_len) - self.max_buffer_size;
            let bytes_to_remove = bytes_to_remove.max(incoming_len); // Remove at least the incoming size

            // Remove old data from the beginning
            if bytes_to_remove < self.data.len() {
                self.data.drain(0..bytes_to_remove);
                self.read_position = self.read_position.saturating_sub(bytes_to_remove);
            } else {
                // Clear entire buffer if we need to remove more than we have
                self.data.clear();
                self.read_position = 0;
            }

            debug!(
                "Circular buffer: removed {} bytes, remaining: {}",
                bytes_to_remove,
                self.data.len()
            );
        }

        self.data.extend_from_slice(data);
        self.write_position += data.len();
        debug!(
            "Written {} bytes to circular buffer, total: {}",
            data.len(),
            self.data.len()
        );
        Ok(())
    }

    fn read_at_position(&self, global_position: usize, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.is_circular {
            self.read_at_position_circular(global_position, buf)
        } else {
            self.read_at_position_linear(global_position, buf)
        }
    }

    fn read_at_position_linear(&self, position: usize, buf: &mut [u8]) -> std::io::Result<usize> {
        if position >= self.data.len() {
            return Ok(0); // EOF
        }

        let available = self.data.len() - position;
        let to_read = buf.len().min(available);

        if to_read > 0 {
            buf[..to_read].copy_from_slice(&self.data[position..position + to_read]);
        }

        Ok(to_read)
    }

    fn read_at_position_circular(
        &self,
        global_position: usize,
        buf: &mut [u8],
    ) -> std::io::Result<usize> {
        // For circular buffer, we need to map global position to buffer position
        let buffer_start_position = self.write_position.saturating_sub(self.data.len());

        if global_position < buffer_start_position {
            // Requested position is before our current buffer window
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Position is before current buffer window",
            ));
        }

        let buffer_position = global_position - buffer_start_position;
        if buffer_position >= self.data.len() {
            return Ok(0); // EOF or beyond current buffer
        }

        let available = self.data.len() - buffer_position;
        let to_read = buf.len().min(available);

        if to_read > 0 {
            buf[..to_read].copy_from_slice(&self.data[buffer_position..buffer_position + to_read]);
        }

        Ok(to_read)
    }

    fn has_enough_data_for_playback(&self, current_position: usize) -> bool {
        if self.is_circular {
            let buffer_start_position = self.write_position.saturating_sub(self.data.len());
            if current_position < buffer_start_position {
                return false; // Position is before our buffer window
            }
            let buffer_position = current_position - buffer_start_position;
            let available_from_position = self.data.len().saturating_sub(buffer_position);
            available_from_position >= self.min_buffer_size
        } else {
            let available_from_position = self.data.len().saturating_sub(current_position);
            available_from_position >= self.min_buffer_size
        }
    }

    fn get_available_bytes(&self) -> usize {
        self.data.len()
    }

    fn get_buffer_info(&self) -> (usize, usize, usize) {
        // Returns (buffer_start_global_position, buffer_size, write_position)
        if self.is_circular {
            let buffer_start = self.write_position.saturating_sub(self.data.len());
            (buffer_start, self.data.len(), self.write_position)
        } else {
            (0, self.data.len(), self.write_position)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::test;

    #[test]
    async fn test_streaming_audio_source_basic() {
        let source = StreamingAudioSource::new(1, 10); // 1MB min, 10MB max

        // Write some test data
        let test_data = b"Hello, world!";
        source.write_data(test_data).await.unwrap();

        // Check if ready for playback (should be false due to min buffer size)
        assert!(!source.is_ready_for_playback().await);

        // Write more data to meet minimum buffer requirement
        let large_data = vec![0u8; 1024 * 1024]; // 1MB
        source.write_data(&large_data).await.unwrap();

        // Now should be ready
        assert!(source.is_ready_for_playback().await);
    }

    #[test]
    async fn test_streaming_read_write() {
        let mut source = StreamingAudioSource::new(0, 10); // 0 min for testing, 10MB max

        let test_data = b"Test streaming data";
        source.write_data(test_data).await.unwrap();

        // Test async read instead of sync read to avoid runtime issues
        let mut read_buf = vec![0u8; test_data.len()];
        let bytes_read = source.async_read(&mut read_buf).await.unwrap();

        assert_eq!(bytes_read, test_data.len());
        assert_eq!(&read_buf[..bytes_read], test_data);
    }
}

/// Buffer recovery actions that can be taken when buffer is insufficient
#[derive(Debug, Clone)]
pub enum BufferRecoveryAction {
    /// No action needed - buffer is sufficient
    NoAction,

    /// Emergency mode for very poor network conditions
    EmergencyMode {
        reduce_quality: bool,
        pause_preloading: bool,
        increase_buffer_target: bool,
    },

    /// Conservative mode for slow networks
    ConservativeMode {
        pause_preloading: bool,
        increase_buffer_target: bool,
    },

    /// Adaptive mode for networks with intermittent issues
    AdaptiveMode {
        adjust_thresholds: bool,
        temporary_buffer_increase: bool,
    },

    /// Minimal intervention for temporary issues
    MinimalMode { wait_duration_ms: u32 },
}
