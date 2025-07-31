use log::{debug, info, warn};
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::{DabError, DabResult};

/// Streaming audio source that supports reading while downloading
#[derive(Clone)]
pub struct StreamingAudioSource {
    buffer: Arc<RwLock<ByteBuffer>>,
    download_complete: Arc<AtomicBool>,
    read_position: Arc<AtomicUsize>,
    total_size: Arc<AtomicUsize>,
}

/// Thread-safe byte buffer for streaming audio data
pub struct ByteBuffer {
    data: Vec<u8>,
    write_position: usize,
    min_buffer_size: usize,
    max_buffer_size: usize,
}

impl StreamingAudioSource {
    /// Create a new streaming audio source
    pub fn new(min_buffer_mb: u32, max_buffer_mb: u32) -> Self {
        let min_buffer_size = (min_buffer_mb * 1024 * 1024) as usize;
        let max_buffer_size = (max_buffer_mb * 1024 * 1024) as usize;
        
        Self {
            buffer: Arc::new(RwLock::new(ByteBuffer::new(min_buffer_size, max_buffer_size))),
            download_complete: Arc::new(AtomicBool::new(false)),
            read_position: Arc::new(AtomicUsize::new(0)),
            total_size: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Write data to the buffer (called by download task)
    pub async fn write_data(&self, data: &[u8]) -> DabResult<()> {
        let mut buffer = self.buffer.write().await;
        buffer.write_data(data)?;
        Ok(())
    }

    /// Mark download as complete
    pub fn mark_complete(&self, total_size: usize) {
        self.total_size.store(total_size, Ordering::Relaxed);
        self.download_complete.store(true, Ordering::Relaxed);
        info!("Download marked complete, total size: {} bytes", total_size);
    }

    /// Check if there's enough data to start playback
    pub async fn is_ready_for_playback(&self) -> bool {
        let buffer = self.buffer.read().await;
        let current_position = self.read_position.load(Ordering::Relaxed);
        buffer.has_enough_data_for_playback(current_position)
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
        buffer.get_available_bytes().saturating_sub(current_position)
    }

    /// Wait for more data to become available
    pub async fn wait_for_data(&self, timeout_ms: u64) -> bool {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);
        
        while start.elapsed() < timeout {
            if self.get_available_bytes().await > 0 || self.is_download_complete() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
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
    fn sync_read_fallback(&self, buf: &mut [u8], current_position: usize) -> std::io::Result<usize> {
        // Simple fallback for sync reads - try to read from buffer if available
        let buffer_guard = match self.buffer.try_read() {
            Ok(guard) => guard,
            Err(_) => return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "Buffer locked"
            ))
        };
        
        let available_bytes = buffer_guard.get_available_bytes();
        if current_position >= available_bytes {
            if self.is_download_complete() {
                return Ok(0); // EOF
            } else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "No data available yet"
                ));
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
                    "Timeout waiting for streaming data"
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
                        "Cannot seek from end: total size unknown"
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
                "Cannot seek beyond available data while streaming"
            ));
        }

        self.read_position.store(new_position, Ordering::Relaxed);
        debug!("Seeked to position {}", new_position);
        Ok(new_position as u64)
    }
}

impl ByteBuffer {
    fn new(min_buffer_size: usize, max_buffer_size: usize) -> Self {
        Self {
            data: Vec::with_capacity(min_buffer_size),
            write_position: 0,
            min_buffer_size,
            max_buffer_size,
        }
    }

    fn write_data(&mut self, data: &[u8]) -> DabResult<()> {
        // Check if adding this data would exceed max buffer size
        if self.data.len() + data.len() > self.max_buffer_size {
            return Err(DabError::Io(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "Buffer overflow: max buffer size exceeded"
            )));
        }

        self.data.extend_from_slice(data);
        self.write_position += data.len();
        debug!("Written {} bytes to buffer, total: {}", data.len(), self.data.len());
        Ok(())
    }

    fn read_at_position(&self, position: usize, buf: &mut [u8]) -> std::io::Result<usize> {
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

    fn has_enough_data_for_playback(&self, current_position: usize) -> bool {
        let available_from_position = self.data.len().saturating_sub(current_position);
        available_from_position >= self.min_buffer_size
    }

    fn get_available_bytes(&self) -> usize {
        self.data.len()
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
        let mut source = StreamingAudioSource::new(0, 10); // 0 min for testing
        
        let test_data = b"Test streaming data";
        source.write_data(test_data).await.unwrap();
        
        // Test async read instead of sync read to avoid runtime issues
        let mut read_buf = vec![0u8; test_data.len()];
        let bytes_read = source.async_read(&mut read_buf).await.unwrap();
        
        assert_eq!(bytes_read, test_data.len());
        assert_eq!(&read_buf[..bytes_read], test_data);
    }
}