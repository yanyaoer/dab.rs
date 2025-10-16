use log::{debug, error, info, warn};
use reqwest::Client;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use super::streaming::StreamingAudioSource;
use super::Track;
use crate::cache::Cache;
use crate::config::{AudioQuality, Config};
use crate::error::{DabError, DabResult};

static CONFIGURED_AUDIO_QUALITY: OnceLock<AudioQuality> = OnceLock::new();

fn configured_audio_quality() -> AudioQuality {
    CONFIGURED_AUDIO_QUALITY
        .get_or_init(|| Config::load().audio_quality)
        .clone()
}

/// Commands for the download manager
#[derive(Debug)]
pub enum DownloadCommand {
    StartDownload {
        track: Track,
        stream_url: String,
        response_tx: mpsc::UnboundedSender<DownloadEvent>,
    },
    CancelDownload {
        track_id: String,
    },
    GetProgress {
        track_id: String,
    },
    CleanupCompleted,
}

/// Events from download tasks
#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Started { track_id: String },
    Progress { track_id: String, progress: f32 },
    StreamReady { track_id: String },
    Completed { track_id: String },
    Failed { track_id: String, error: String },
    BufferingStart { track_id: String },
    BufferingEnd { track_id: String },
}

/// Active download task information
pub struct DownloadTask {
    pub track: Track,
    pub stream_url: String,
    pub streaming_source: Arc<StreamingAudioSource>,
    pub progress: Arc<AtomicU32>, // Progress as percentage * 100 (0-10000)
    pub cancel_token: CancellationToken,
    pub started_at: Instant,
    pub bytes_downloaded: Arc<AtomicU32>,
    pub total_bytes: Arc<AtomicU32>,
}

/// Asynchronous download manager
pub struct AsyncDownloadManager {
    active_downloads: Arc<RwLock<HashMap<String, DownloadTask>>>,
    completed_downloads: Arc<RwLock<HashMap<String, Arc<StreamingAudioSource>>>>,
    http_client: Client,
    cache: Arc<RwLock<Cache>>,
    command_tx: mpsc::UnboundedSender<DownloadCommand>,
    _manager_handle: tokio::task::JoinHandle<()>,
}

impl AsyncDownloadManager {
    pub async fn new(cache: Cache) -> DabResult<Self> {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let active_downloads = Arc::new(RwLock::new(HashMap::new()));
        let completed_downloads = Arc::new(RwLock::new(HashMap::new()));
        let http_client = Client::builder().timeout(Duration::from_secs(30)).build()?;
        let cache_arc = Arc::new(RwLock::new(cache));

        let manager_handle = {
            let active_downloads = active_downloads.clone();
            let completed_downloads = completed_downloads.clone();
            let http_client = http_client.clone();
            let cache = cache_arc.clone();

            tokio::spawn(async move {
                Self::run_manager(
                    command_rx,
                    active_downloads,
                    completed_downloads,
                    http_client,
                    cache,
                )
                .await;
            })
        };

        Ok(Self {
            active_downloads,
            completed_downloads,
            http_client,
            cache: cache_arc,
            command_tx,
            _manager_handle: manager_handle,
        })
    }

    /// Start downloading a track
    pub async fn start_download(
        &self,
        track: Track,
        stream_url: String,
    ) -> DabResult<(
        Arc<StreamingAudioSource>,
        mpsc::UnboundedReceiver<DownloadEvent>,
    )> {
        // Check if already downloading or completed
        if let Some(existing_task) = self.active_downloads.read().await.get(&track.id) {
            info!("Track {} already downloading", track.id);
            return Ok((
                existing_task.streaming_source.clone(),
                mpsc::unbounded_channel().1,
            ));
        }

        if let Some(completed_source) = self.completed_downloads.read().await.get(&track.id) {
            info!("Track {} already downloaded", track.id);
            return Ok((completed_source.clone(), mpsc::unbounded_channel().1));
        }

        let (event_tx, event_rx) = mpsc::unbounded_channel();

        self.command_tx
            .send(DownloadCommand::StartDownload {
                track: track.clone(),
                stream_url,
                response_tx: event_tx,
            })
            .map_err(|_| DabError::Player("Failed to send download command".to_string()))?;

        // Wait for the download task to be created and return streaming source
        let streaming_source = loop {
            tokio::time::sleep(Duration::from_millis(10)).await;
            if let Some(task) = self.active_downloads.read().await.get(&track.id) {
                break task.streaming_source.clone();
            }
        };

        Ok((streaming_source, event_rx))
    }

    /// Cancel a download
    pub async fn cancel_download(&self, track_id: &str) -> DabResult<()> {
        self.command_tx
            .send(DownloadCommand::CancelDownload {
                track_id: track_id.to_string(),
            })
            .map_err(|_| DabError::Player("Failed to send cancel command".to_string()))?;
        Ok(())
    }

    /// Get download progress for a track
    pub async fn get_progress(&self, track_id: &str) -> Option<f32> {
        if let Some(task) = self.active_downloads.read().await.get(track_id) {
            Some(task.progress.load(Ordering::Relaxed) as f32 / 100.0)
        } else {
            None
        }
    }

    /// Check if track is ready for streaming playback
    pub async fn is_stream_ready(&self, track_id: &str) -> bool {
        if let Some(_completed) = self.completed_downloads.read().await.get(track_id) {
            return true;
        }

        if let Some(task) = self.active_downloads.read().await.get(track_id) {
            return task.streaming_source.is_ready_for_playback().await;
        }

        false
    }

    /// Get streaming source for a track (if available)
    pub async fn get_streaming_source(&self, track_id: &str) -> Option<Arc<StreamingAudioSource>> {
        // Check completed downloads first
        if let Some(source) = self.completed_downloads.read().await.get(track_id) {
            return Some(source.clone());
        }

        // Check active downloads
        if let Some(task) = self.active_downloads.read().await.get(track_id) {
            return Some(task.streaming_source.clone());
        }

        None
    }

    /// Internal manager task
    async fn run_manager(
        mut command_rx: mpsc::UnboundedReceiver<DownloadCommand>,
        active_downloads: Arc<RwLock<HashMap<String, DownloadTask>>>,
        completed_downloads: Arc<RwLock<HashMap<String, Arc<StreamingAudioSource>>>>,
        http_client: Client,
        cache: Arc<RwLock<Cache>>,
    ) {
        info!("Download manager started");

        while let Some(command) = command_rx.recv().await {
            match command {
                DownloadCommand::StartDownload {
                    track,
                    stream_url,
                    response_tx,
                } => {
                    let task = Self::create_download_task(
                        track.clone(),
                        stream_url,
                        http_client.clone(),
                        cache.clone(),
                        response_tx,
                    )
                    .await;

                    if let Ok(task) = task {
                        active_downloads
                            .write()
                            .await
                            .insert(track.id.clone(), task);
                        info!("Started download for track: {}", track.id);
                    }
                }
                DownloadCommand::CancelDownload { track_id } => {
                    if let Some(task) = active_downloads.write().await.remove(&track_id) {
                        task.cancel_token.cancel();
                        info!("Cancelled download for track: {}", track_id);
                    }
                }
                DownloadCommand::CleanupCompleted => {
                    Self::cleanup_completed_downloads(&active_downloads, &completed_downloads)
                        .await;
                }
                _ => {}
            }
        }

        info!("Download manager stopped");
    }

    /// Create and start a download task
    async fn create_download_task(
        track: Track,
        stream_url: String,
        http_client: Client,
        cache: Arc<RwLock<Cache>>,
        event_tx: mpsc::UnboundedSender<DownloadEvent>,
    ) -> DabResult<DownloadTask> {
        // Load config to check streaming buffer setting
        let config = Config::load();
        let download_complete_before_play = config.streaming_buffer == 0;

        // Create streaming source with optimized buffer sizes based on track properties
        let is_lossless = Self::is_lossless_stream(&track, &stream_url);

        let streaming_source = if download_complete_before_play {
            // Download complete mode: Use larger buffer to store entire file
            info!("Using download-complete mode for track: {} (will download entire file before playback)", track.id);
            // Use large buffer to store the complete file
            Arc::new(StreamingAudioSource::new(50, 500)) // 50MB min, 500MB max for complete file storage
        } else {
            // Normal streaming mode with configured buffer size
            let (min_buffer_mb, max_buffer_mb) =
                Self::calculate_optimal_buffer_sizes(&track, is_lossless, config.streaming_buffer);
            Arc::new(StreamingAudioSource::new(min_buffer_mb, max_buffer_mb))
        };

        // Estimate bitrate if available from track metadata
        if let Some(estimated_bitrate) = Self::estimate_track_bitrate(&track, is_lossless) {
            streaming_source
                .set_estimated_bitrate(estimated_bitrate)
                .await;
        }

        if is_lossless && !download_complete_before_play {
            streaming_source.apply_lossless_profile().await;
        }

        let cancel_token = CancellationToken::new();
        let progress = Arc::new(AtomicU32::new(0));
        let bytes_downloaded = Arc::new(AtomicU32::new(0));
        let total_bytes = Arc::new(AtomicU32::new(0));

        // Start the actual download task
        let _download_task = {
            let track = track.clone();
            let stream_url = stream_url.clone();
            let streaming_source = streaming_source.clone();
            let cancel_token = cancel_token.clone();
            let progress = progress.clone();
            let bytes_downloaded = bytes_downloaded.clone();
            let total_bytes = total_bytes.clone();
            let event_tx = event_tx.clone();

            tokio::spawn(async move {
                if let Err(e) = Self::download_track(
                    &track,
                    &stream_url,
                    &streaming_source,
                    &cancel_token,
                    &progress,
                    &bytes_downloaded,
                    &total_bytes,
                    http_client,
                    cache,
                    event_tx.clone(),
                    download_complete_before_play,
                )
                .await
                {
                    error!("Download failed for track {}: {}", track.id, e);
                    let _ = event_tx.send(DownloadEvent::Failed {
                        track_id: track.id,
                        error: e.to_string(),
                    });
                }
            })
        };

        Ok(DownloadTask {
            track,
            stream_url,
            streaming_source,
            progress,
            cancel_token,
            started_at: Instant::now(),
            bytes_downloaded,
            total_bytes,
        })
    }

    /// Download track data and stream to buffer
    async fn download_track(
        track: &Track,
        stream_url: &str,
        streaming_source: &Arc<StreamingAudioSource>,
        cancel_token: &CancellationToken,
        progress: &Arc<AtomicU32>,
        bytes_downloaded: &Arc<AtomicU32>,
        total_bytes: &Arc<AtomicU32>,
        http_client: Client,
        cache: Arc<RwLock<Cache>>,
        event_tx: mpsc::UnboundedSender<DownloadEvent>,
        download_complete_before_play: bool,
    ) -> DabResult<()> {
        let _ = event_tx.send(DownloadEvent::Started {
            track_id: track.id.clone(),
        });

        debug!(
            "Starting HTTP download for {} from {}",
            track.id, stream_url
        );
        let response = http_client.get(stream_url).send().await?;
        let response = response.error_for_status()?;

        let content_length = response.content_length().unwrap_or(0);
        total_bytes.store(content_length as u32, Ordering::Relaxed);

        let mut stream = response.bytes_stream();
        let mut temp_cache_data = Vec::new();
        let mut downloaded = 0u64;
        let mut stream_ready_sent = false;

        info!(
            "Starting download for track {} ({} bytes, download_complete_before_play={})",
            track.id, content_length, download_complete_before_play
        );

        // In download-complete mode, we'll send StreamReady only after download completes
        if download_complete_before_play {
            info!(
                "Download-complete mode: Will signal stream ready after full download for track {}",
                track.id
            );
        }

        use futures_util::StreamExt;

        while let Some(chunk_result) = stream.next().await {
            // Check for cancellation
            if cancel_token.is_cancelled() {
                info!("Download cancelled for track: {}", track.id);
                return Ok(());
            }

            let chunk = chunk_result?;

            // Write to streaming buffer
            streaming_source.write_data(&chunk).await?;

            // Also collect for caching (always collect in download-complete mode)
            if download_complete_before_play || true {
                // Always cache for now
                temp_cache_data.extend_from_slice(&chunk);
            }

            downloaded += chunk.len() as u64;
            bytes_downloaded.store(downloaded as u32, Ordering::Relaxed);

            // Update progress
            let progress_percent = if content_length > 0 {
                ((downloaded as f32 / content_length as f32) * 10000.0) as u32
            } else {
                0
            };
            progress.store(progress_percent, Ordering::Relaxed);

            // Send progress event
            let _ = event_tx.send(DownloadEvent::Progress {
                track_id: track.id.clone(),
                progress: progress_percent as f32 / 100.0,
            });

            // Check if ready for streaming (skip in download-complete mode)
            if !download_complete_before_play && !stream_ready_sent {
                // Use the streaming source's own readiness check which includes format detection requirements
                if streaming_source.is_ready_for_playback().await {
                    let _ = event_tx.send(DownloadEvent::StreamReady {
                        track_id: track.id.clone(),
                    });
                    stream_ready_sent = true;

                    let available_ms = streaming_source.get_available_playback_ms().await;
                    info!(
                        "Stream ready for track: {} ({} bytes buffered, ~{}ms available)",
                        track.id, downloaded, available_ms
                    );
                } else {
                    // For debugging, show why it's not ready yet
                    let config = streaming_source.get_adaptive_config().await;
                    let min_buffer_bytes = streaming_source
                        .calculate_buffer_bytes_for_duration(config.min_playback_buffer_ms)
                        .await;
                    let min_detection_bytes = 65536; // Same as in is_ready_for_playback
                    let required_bytes = min_buffer_bytes.max(min_detection_bytes);

                    if downloaded >= required_bytes as u64 {
                        debug!(
                            "Downloaded {} bytes (>= {}), checking stream readiness...",
                            downloaded, required_bytes
                        );
                    }
                }
            }

            debug!(
                "Downloaded {} / {} bytes for track {}",
                downloaded, content_length, track.id
            );
        }

        // Mark download as complete
        streaming_source.mark_complete(downloaded as usize);

        // In download-complete mode, send StreamReady event after download completes
        if download_complete_before_play {
            let _ = event_tx.send(DownloadEvent::StreamReady {
                track_id: track.id.clone(),
            });
            info!(
                "Download-complete mode: Track {} is now ready for playback ({} bytes downloaded)",
                track.id, downloaded
            );
        }

        // Cache the complete file with full API metadata
        if !temp_cache_data.is_empty() {
            let temp_file = tempfile::NamedTempFile::new()?;
            tokio::fs::write(temp_file.path(), &temp_cache_data).await?;

            let mut cache_lock = cache.write().await;
            if let Err(e) = cache_lock
                .store_track_with_api_metadata(track, temp_file.path(), stream_url)
                .await
            {
                warn!("Failed to cache track {}: {}", track.id, e);
            } else {
                info!(
                    "Cached track with API metadata: {} by {} ({} bytes)",
                    track.title,
                    track.artist,
                    temp_cache_data.len()
                );
            }
        }

        let _ = event_tx.send(DownloadEvent::Completed {
            track_id: track.id.clone(),
        });
        info!(
            "Download completed for track: {} ({} bytes)",
            track.id, downloaded
        );

        Ok(())
    }

    /// Clean up completed downloads that are no longer needed
    async fn cleanup_completed_downloads(
        active_downloads: &Arc<RwLock<HashMap<String, DownloadTask>>>,
        completed_downloads: &Arc<RwLock<HashMap<String, Arc<StreamingAudioSource>>>>,
    ) {
        let mut active = active_downloads.write().await;
        let mut completed = completed_downloads.write().await;

        // Move completed downloads from active to completed
        let mut to_move = Vec::new();
        for (track_id, task) in active.iter() {
            if task.streaming_source.is_download_complete() {
                to_move.push((track_id.clone(), task.streaming_source.clone()));
            }
        }

        for (track_id, source) in to_move {
            active.remove(&track_id);
            completed.insert(track_id.clone(), source);
            debug!("Moved completed download to completed list: {}", track_id);
        }

        // TODO: Add logic to remove old completed downloads based on LRU or time
    }

    /// Calculate optimal buffer sizes based on track properties
    fn calculate_optimal_buffer_sizes(
        track: &Track,
        lossless: bool,
        streaming_buffer: u32,
    ) -> (u32, u32) {
        // If streaming_buffer is specified, use it as both min and max
        if streaming_buffer > 0 {
            let configured_buffer = streaming_buffer.min(512); // Cap at 512MB
            return (configured_buffer / 2, configured_buffer);
        }

        // Default logic for automatic buffer sizing
        let (mut min_buffer_mb, mut max_buffer_mb) = if lossless { (12, 256) } else { (5, 100) };

        if track.duration_ms > 0 {
            let duration_seconds = track.duration_ms / 1000;
            if duration_seconds < 60 {
                if lossless {
                    min_buffer_mb = 8;
                    max_buffer_mb = 160;
                } else {
                    min_buffer_mb = 3;
                    max_buffer_mb = 50;
                }
            } else if duration_seconds > 600 {
                if lossless {
                    min_buffer_mb = 20;
                    max_buffer_mb = 320;
                } else {
                    min_buffer_mb = 10;
                    max_buffer_mb = 200;
                }
            }
        }

        max_buffer_mb = max_buffer_mb.max(min_buffer_mb);

        debug!(
            "Calculated buffer sizes for track {}: {}MB min, {}MB max (lossless={}, configured={})",
            track.id, min_buffer_mb, max_buffer_mb, lossless, streaming_buffer
        );

        (min_buffer_mb, max_buffer_mb)
    }
    fn is_lossless_stream(track: &Track, stream_url: &str) -> bool {
        let quality = configured_audio_quality();
        if matches!(quality, AudioQuality::High) {
            return true;
        }

        let url = stream_url.to_lowercase();
        if url.contains("flac")
            || url.contains("lossless")
            || url.contains("hires")
            || url.contains("wav")
        {
            return true;
        }

        let title = track.title.to_lowercase();
        title.contains("flac") || title.contains("lossless") || title.contains("wav")
    }

    /// Estimate track bitrate from metadata
    fn estimate_track_bitrate(track: &Track, lossless: bool) -> Option<u32> {
        if lossless {
            // Assume at least CD-quality lossless (16-bit/44.1kHz stereo) or higher
            return Some(2000);
        }

        let title = track.title.to_lowercase();
        if title.contains("flac") {
            Some(1000)
        } else if title.contains("wav") {
            Some(1411)
        } else {
            Some(320)
        }
    }
}

impl Drop for AsyncDownloadManager {
    fn drop(&mut self) {
        // Cancel all active downloads
        let active_downloads = self.active_downloads.clone();
        tokio::spawn(async move {
            let downloads = active_downloads.read().await;
            for task in downloads.values() {
                task.cancel_token.cancel();
            }
        });
    }
}
