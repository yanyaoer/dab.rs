use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use super::streaming::StreamingAudioSource;
use crate::error::{DabError, DabResult};

// Create a trait that combines Read + Seek + Send + Sync
pub trait ReadSeek: Read + Seek + Send + Sync {}
impl<T: Read + Seek + Send + Sync> ReadSeek for T {}

pub struct AudioDecoder {
    format: Box<dyn FormatReader>,
    decoder: Option<Box<dyn Decoder>>,
    track_id: u32,
    sample_rate: u32,
    channels: u16,
    is_streaming: bool,
}

// Simple wrapper to make ReadSeek compatible with MediaSource
struct ReadSeekWrapper {
    inner: Box<dyn ReadSeek>,
}

impl ReadSeekWrapper {
    fn new(reader: Box<dyn ReadSeek>) -> Self {
        Self { inner: reader }
    }
}

impl Read for ReadSeekWrapper {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for ReadSeekWrapper {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl symphonia::core::io::MediaSource for ReadSeekWrapper {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

// Wrapper for streaming audio source
struct StreamingWrapper {
    source: Arc<StreamingAudioSource>,
}

impl StreamingWrapper {
    fn new(source: Arc<StreamingAudioSource>) -> Self {
        Self { source }
    }
}

impl Read for StreamingWrapper {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use log::{debug, trace, warn};

        let mut source_clone = (*self.source).clone();
        let mut attempts: u32 = 0;
        let mut total_wait_ms = 0u64;
        let buf_size = buf.len();

        // Determine if this is likely a high bitrate stream by checking buffer config
        // We can't directly access the adaptive config from here, so we use conservative defaults
        // For lossless/high bitrate, we should be more patient
        let is_high_bitrate = buf_size > 8192; // Larger buffer requests often indicate high bitrate

        // Log the read request size to help diagnose issues
        if buf_size > 16384 {
            debug!("StreamingWrapper: Large read request of {} bytes", buf_size);
        }

        // Adaptive retry strategy based on likely bitrate
        let max_initial_attempts = if is_high_bitrate { 15 } else { 8 };
        let initial_backoff_base = if is_high_bitrate { 5 } else { 3 };
        let max_blocking_wait_ms = if is_high_bitrate { 2000 } else { 1000 };
        let max_total_wait_ms = if is_high_bitrate { 5000 } else { 2500 };

        loop {
            match source_clone.read(buf) {
                Ok(bytes_read) => {
                    if bytes_read > 0 {
                        if total_wait_ms > 100 {
                            debug!("StreamingWrapper: Read {} bytes after {}ms wait (buffer request was {} bytes)",
                                   bytes_read, total_wait_ms, buf_size);
                        } else {
                            trace!(
                                "StreamingWrapper: Read {} bytes after {}ms total wait",
                                bytes_read,
                                total_wait_ms
                            );
                        }
                    } else {
                        debug!(
                            "StreamingWrapper: EOF reached (download_complete={})",
                            self.source.is_download_complete()
                        );
                    }
                    return Ok(bytes_read);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // If download is complete and no data available, it's EOF
                    if self.source.is_download_complete() {
                        debug!("StreamingWrapper: Download complete, returning EOF");
                        return Ok(0);
                    }

                    // Phase 1: Quick retries with exponential backoff
                    if attempts < max_initial_attempts {
                        // Use a more gradual backoff for high bitrate streams
                        let backoff_ms = if is_high_bitrate {
                            // For high bitrate: 5ms, 10ms, 20ms, 40ms, 80ms, 160ms...
                            (initial_backoff_base * (1u64 << attempts.min(7))).min(200)
                        } else {
                            // For normal: 3ms, 6ms, 12ms, 24ms, 48ms, 96ms
                            (initial_backoff_base * (1u64 << attempts.min(5))).min(100)
                        };

                        trace!(
                            "StreamingWrapper: Retry {} with {}ms backoff (request size: {} bytes)",
                            attempts + 1,
                            backoff_ms,
                            buf_size
                        );
                        std::thread::sleep(std::time::Duration::from_millis(backoff_ms));
                        total_wait_ms += backoff_ms;
                        attempts += 1;
                        continue;
                    }

                    // Phase 2: Blocking wait with larger timeout
                    // This is especially important for high bitrate streams where
                    // the download thread might need more time to fetch data
                    if total_wait_ms < max_total_wait_ms {
                        let remaining_wait = max_total_wait_ms - total_wait_ms;
                        let blocking_wait = remaining_wait.min(max_blocking_wait_ms);

                        debug!("StreamingWrapper: Attempting blocking wait for {}ms (total waited: {}ms, request: {} bytes)",
                               blocking_wait, total_wait_ms, buf_size);

                        if self
                            .source
                            .blocking_wait_for_data(std::time::Duration::from_millis(blocking_wait))
                        {
                            // Data became available, reset attempts for next phase
                            debug!("StreamingWrapper: Data available after blocking wait");
                            attempts = 0;
                            total_wait_ms += blocking_wait;
                            continue;
                        }

                        total_wait_ms += blocking_wait;
                        debug!(
                            "StreamingWrapper: Still no data after {}ms blocking wait",
                            blocking_wait
                        );
                    }

                    // Phase 3: Final attempt with longer blocking wait for high bitrate
                    if is_high_bitrate && total_wait_ms < max_total_wait_ms * 2 {
                        warn!("StreamingWrapper: High bitrate stream experiencing buffer underrun - attempting recovery (request: {} bytes)", buf_size);

                        // One last patient wait for high bitrate streams
                        if self
                            .source
                            .blocking_wait_for_data(std::time::Duration::from_millis(2000))
                        {
                            debug!("StreamingWrapper: Recovery successful - data now available");
                            total_wait_ms += 2000;
                            continue;
                        }
                        warn!(
                            "StreamingWrapper: Recovery failed - still no data after extended wait"
                        );
                    }

                    // Check one more time if download completed during our waits
                    if self.source.is_download_complete() {
                        debug!("StreamingWrapper: Download completed during wait, checking for remaining data");
                        // Try one more read in case data became available
                        match source_clone.read(buf) {
                            Ok(bytes_read) => return Ok(bytes_read),
                            Err(_) => return Ok(0), // True EOF
                        }
                    }

                    warn!("StreamingWrapper: BUFFER UNDERRUN - Data not available after {}ms total wait (high_bitrate={}, request_size={} bytes)",
                           total_wait_ms, is_high_bitrate, buf_size);

                    // For streaming sources, we should be more patient and not return WouldBlock immediately
                    // This helps prevent audio artifacts (pops/clicks) during playback
                    if self.source.is_download_complete() {
                        // If download is complete but no data, it's true EOF
                        return Ok(0);
                    }

                    // One final aggressive wait before giving up
                    warn!(
                        "StreamingWrapper: Final blocking wait of 100ms to prevent audio artifacts"
                    );
                    if self
                        .source
                        .blocking_wait_for_data(std::time::Duration::from_millis(100))
                    {
                        debug!("StreamingWrapper: Data available after final wait");
                        // Try to read again
                        match source_clone.read(buf) {
                            Ok(bytes_read) => {
                                debug!(
                                    "StreamingWrapper: Successfully read {} bytes after final wait",
                                    bytes_read
                                );
                                return Ok(bytes_read);
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                // Still no data, continue to return WouldBlock
                            }
                            Err(e) => return Err(e),
                        }
                    }

                    // Return WouldBlock to allow decoder to handle the situation
                    // This should be rare with the aggressive retry logic above
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::WouldBlock,
                        format!(
                            "Stream buffer underrun after {}ms wait for {} bytes",
                            total_wait_ms + 100,
                            buf_size
                        ),
                    ));
                }
                Err(e) => {
                    warn!("StreamingWrapper: Read error: {}", e);
                    return Err(e);
                }
            }
        }
    }
}

impl Seek for StreamingWrapper {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let mut source_clone = (*self.source).clone();
        source_clone.seek(pos)
    }
}

impl symphonia::core::io::MediaSource for StreamingWrapper {
    fn is_seekable(&self) -> bool {
        // Streaming sources have limited seeking capability
        self.source.is_download_complete()
    }

    fn byte_len(&self) -> Option<u64> {
        None // Unknown until download is complete
    }
}

impl AudioDecoder {
    pub fn new(_source: Box<dyn Read + Send + Sync>) -> DabResult<Self> {
        // For now, we'll require sources to also implement Seek
        // In a real implementation, we'd handle non-seekable sources differently
        Err(DabError::Decode(
            "Audio decoder requires seekable source".to_string(),
        ))
    }

    pub fn from_seekable(source: Box<dyn ReadSeek>) -> DabResult<Self> {
        use log::info;
        info!("AudioDecoder::from_seekable - creating decoder WITHOUT StreamingWrapper");

        let wrapped_source = ReadSeekWrapper::new(source);
        let mss = MediaSourceStream::new(Box::new(wrapped_source), Default::default());
        let hint = Hint::new();

        let meta_opts: MetadataOptions = Default::default();
        let fmt_opts: FormatOptions = Default::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &fmt_opts, &meta_opts)
            .map_err(|e| DabError::Decode(format!("Failed to probe format: {}", e)))?;

        let format = probed.format;

        // Find the default (first) track
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| DabError::Decode("No supported audio tracks found".to_string()))?;

        let track_id = track.id;
        let codec_params = &track.codec_params;

        // Create decoder
        let decoder = symphonia::default::get_codecs()
            .make(codec_params, &DecoderOptions::default())
            .map_err(|e| DabError::Decode(format!("Failed to create decoder: {}", e)))?;

        let sample_rate = codec_params.sample_rate.unwrap_or(44100);
        let channels = codec_params
            .channels
            .map(|ch| ch.count() as u16)
            .unwrap_or(2);

        Ok(Self {
            format,
            decoder: Some(decoder),
            track_id,
            sample_rate,
            channels,
            is_streaming: false,
        })
    }

    /// Create decoder from streaming source
    pub fn from_streaming(source: Arc<StreamingAudioSource>) -> DabResult<Self> {
        use log::warn;
        warn!("AudioDecoder::from_streaming - creating decoder WITH StreamingWrapper (THIS SHOULD NOT BE CALLED when streaming_buffer=0)");

        let wrapped_source = StreamingWrapper::new(source);
        let mss = MediaSourceStream::new(Box::new(wrapped_source), Default::default());
        let hint = Hint::new();

        let meta_opts: MetadataOptions = Default::default();
        let fmt_opts: FormatOptions = Default::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &fmt_opts, &meta_opts)
            .map_err(|e| DabError::Decode(format!("Failed to probe streaming format: {}", e)))?;

        let format = probed.format;

        // Find the default (first) track
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| {
                DabError::Decode("No supported audio tracks found in stream".to_string())
            })?;

        let track_id = track.id;
        let codec_params = &track.codec_params;

        // Create decoder
        let decoder = symphonia::default::get_codecs()
            .make(codec_params, &DecoderOptions::default())
            .map_err(|e| DabError::Decode(format!("Failed to create streaming decoder: {}", e)))?;

        let sample_rate = codec_params.sample_rate.unwrap_or(44100);
        let channels = codec_params
            .channels
            .map(|ch| ch.count() as u16)
            .unwrap_or(2);

        Ok(Self {
            format,
            decoder: Some(decoder),
            track_id,
            sample_rate,
            channels,
            is_streaming: true,
        })
    }

    pub fn next_frame(&mut self) -> DabResult<Option<Vec<f32>>> {
        use log::{debug, trace, warn};

        let packet = match self.format.next_packet() {
            Ok(packet) => {
                trace!("Successfully read packet for track {}", packet.track_id());
                packet
            }
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                debug!("next_frame: Reached EOF, returning None");
                return Ok(None);
            }
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                // This is a temporary condition during streaming - signal it specially
                // Don't log as error/warn as this is expected during streaming
                trace!("next_frame: Temporary buffer underrun (WouldBlock)");
                return Err(DabError::TemporaryBufferUnderrun);
            }
            Err(e) => {
                warn!("next_frame: Failed to read packet: {}", e);
                return Err(DabError::Decode(format!("Failed to read packet: {}", e)));
            }
        };

        // Skip packets that don't belong to our track
        if packet.track_id() != self.track_id {
            debug!(
                "Skipping packet for track {} (looking for {})",
                packet.track_id(),
                self.track_id
            );
            return self.next_frame();
        }

        let decoder = self
            .decoder
            .as_mut()
            .ok_or_else(|| DabError::Decode("Decoder not initialized".to_string()))?;

        let decoded = decoder.decode(&packet).map_err(|e| {
            warn!("next_frame: Failed to decode packet: {}", e);
            DabError::Decode(format!("Failed to decode packet: {}", e))
        })?;

        // Convert to f32 samples
        let spec = decoded.spec();
        let duration = decoded.capacity() as u64;

        let mut sample_buffer = SampleBuffer::<f32>::new(duration, *spec);
        sample_buffer.copy_interleaved_ref(decoded);

        let samples = sample_buffer.samples().to_vec();
        debug!("next_frame: Decoded {} samples", samples.len());
        Ok(Some(samples))
    }

    pub fn seek(&mut self, position_ms: u32) -> DabResult<()> {
        // Don't seek on streaming sources unless download is complete
        if self.is_streaming {
            return Err(DabError::Decode(
                "Seeking not supported on streaming sources".to_string(),
            ));
        }

        let time_base = self
            .format
            .default_track()
            .ok_or_else(|| DabError::Decode("No default track".to_string()))?
            .codec_params
            .time_base;

        if let Some(time_base) = time_base {
            let timestamp =
                (position_ms as u64 * time_base.numer as u64) / (1000 * time_base.denom as u64);

            let seek_to = SeekTo::Time {
                time: Time::new(timestamp, 0.0),
                track_id: Some(self.track_id),
            };
            self.format
                .seek(SeekMode::Accurate, seek_to)
                .map_err(|e| DabError::Decode(format!("Seek failed: {}", e)))?;
        }

        Ok(())
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn duration_ms(&self) -> Option<u32> {
        // TODO: Extract duration from metadata
        None
    }

    pub fn is_streaming(&self) -> bool {
        self.is_streaming
    }
}
