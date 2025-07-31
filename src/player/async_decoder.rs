use log::{debug, info, warn};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::decoder::AudioDecoder;
use super::streaming::StreamingAudioSource;
use crate::error::{DabError, DabResult};

/// Asynchronous audio decoder that can handle streaming sources
pub struct AsyncAudioDecoder {
    decoder: AudioDecoder,
    streaming_source: Option<Arc<StreamingAudioSource>>,
    sample_rate: u32,
    channels: u16,
    _decode_task: Option<JoinHandle<()>>,
}

/// Audio frame with decoded samples
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl AsyncAudioDecoder {
    /// Create from streaming source
    pub async fn from_streaming(
        streaming_source: Arc<StreamingAudioSource>,
    ) -> DabResult<(Self, mpsc::UnboundedReceiver<AudioFrame>)> {
        // Wait for enough data to start decoding
        let mut retries = 0;
        while !streaming_source.is_ready_for_playback().await && retries < 50 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            retries += 1;
        }

        if !streaming_source.is_ready_for_playback().await {
            return Err(DabError::Decode(
                "Streaming source not ready for decoding".to_string(),
            ));
        }

        // Create a dummy decoder to get format info
        // We'll use a non-blocking reader approach
        let reader = StreamingReaderAdapter::new(streaming_source.clone());
        let decoder = AudioDecoder::from_seekable(Box::new(reader))?;

        let sample_rate = decoder.sample_rate();
        let channels = decoder.channels();

        let (frame_tx, frame_rx) = mpsc::unbounded_channel();

        // Start decode task
        let decode_task = {
            let streaming_source = streaming_source.clone();
            let frame_tx = frame_tx.clone();
            
            tokio::spawn(async move {
                Self::decode_streaming_audio(streaming_source, frame_tx).await;
            })
        };

        Ok((
            Self {
                decoder,
                streaming_source: Some(streaming_source),
                sample_rate,
                channels,
                _decode_task: Some(decode_task),
            },
            frame_rx,
        ))
    }

    /// Create from traditional seekable source
    pub fn from_seekable(decoder: AudioDecoder) -> Self {
        let sample_rate = decoder.sample_rate();
        let channels = decoder.channels();

        Self {
            decoder,
            streaming_source: None,
            sample_rate,
            channels,
            _decode_task: None,
        }
    }

    /// Get next frame (for seekable sources)
    pub fn next_frame(&mut self) -> DabResult<Option<Vec<f32>>> {
        if self.streaming_source.is_some() {
            return Err(DabError::Decode(
                "Use frame receiver for streaming sources".to_string(),
            ));
        }
        self.decoder.next_frame()
    }

    /// Async decode task for streaming sources
    async fn decode_streaming_audio(
        streaming_source: Arc<StreamingAudioSource>,
        frame_tx: mpsc::UnboundedSender<AudioFrame>,
    ) {
        info!("Starting streaming decode task");

        // Simplified streaming decode - in practice this would need more sophisticated buffering
        let mut decode_buffer = Vec::new();
        let chunk_size = 8192; // 8KB chunks

        loop {
            // Wait for data
            if streaming_source.get_available_bytes().await == 0 
                && !streaming_source.is_download_complete() 
            {
                if !streaming_source.wait_for_data(1000).await {
                    debug!("No more data available, ending decode task");
                    break;
                }
            }

            // Try to read a chunk
            let mut chunk = vec![0u8; chunk_size];
            let bytes_read = match streaming_source.clone().async_read(&mut chunk).await {
                Ok(n) if n > 0 => n,
                Ok(0) => {
                    // EOF or no data
                    if streaming_source.is_download_complete() {
                        debug!("Streaming decode completed (EOF)");
                        break;
                    } else {
                        // Wait for more data
                        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                        continue;
                    }
                }
                Err(e) => {
                    warn!("Error reading from streaming source: {}", e);
                    break;
                }
            };

            chunk.truncate(bytes_read);
            decode_buffer.extend_from_slice(&chunk);

            // Simple frame creation (in practice, this would use proper audio decoding)
            if decode_buffer.len() >= 4096 {
                // Convert bytes to f32 samples (simplified)
                let samples: Vec<f32> = decode_buffer
                    .chunks(4)
                    .take(1024) // 1024 samples per frame
                    .map(|chunk| {
                        let mut bytes = [0u8; 4];
                        bytes.copy_from_slice(&chunk[..4.min(chunk.len())]);
                        f32::from_le_bytes(bytes)
                    })
                    .collect();

                let frame = AudioFrame {
                    samples,
                    sample_rate: 44100, // Default
                    channels: 2,        // Default stereo
                };

                if frame_tx.send(frame).is_err() {
                    debug!("Frame receiver dropped, ending decode task");
                    break;
                }

                // Remove processed data
                decode_buffer.drain(..4096);
            }

            // Yield to other tasks
            tokio::task::yield_now().await;
        }

        info!("Streaming decode task completed");
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming_source.is_some()
    }
}

/// Adapter to make StreamingAudioSource work with traditional Read+Seek
struct StreamingReaderAdapter {
    source: Arc<StreamingAudioSource>,
}

impl StreamingReaderAdapter {
    fn new(source: Arc<StreamingAudioSource>) -> Self {
        Self { source }
    }
}

impl std::io::Read for StreamingReaderAdapter {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // This is a simplified adapter - in practice might need more sophisticated handling
        let mut source_clone = (*self.source).clone();
        source_clone.read(buf)
    }
}

impl std::io::Seek for StreamingReaderAdapter {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        let mut source_clone = (*self.source).clone();
        source_clone.seek(pos)
    }
}

impl super::decoder::ReadSeek for StreamingReaderAdapter {}