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
        // Clone the source for async operation
        let mut source_clone = (*self.source).clone();
        source_clone.read(buf)
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
            .ok_or_else(|| DabError::Decode("No supported audio tracks found in stream".to_string()))?;

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
        let packet = match self.format.next_packet() {
            Ok(packet) => packet,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                return Ok(None);
            }
            Err(e) => return Err(DabError::Decode(format!("Failed to read packet: {}", e))),
        };

        // Skip packets that don't belong to our track
        if packet.track_id() != self.track_id {
            return self.next_frame();
        }

        let decoder = self
            .decoder
            .as_mut()
            .ok_or_else(|| DabError::Decode("Decoder not initialized".to_string()))?;

        let decoded = decoder
            .decode(&packet)
            .map_err(|e| DabError::Decode(format!("Failed to decode packet: {}", e)))?;

        // Convert to f32 samples
        let spec = decoded.spec();
        let duration = decoded.capacity() as u64;

        let mut sample_buffer = SampleBuffer::<f32>::new(duration, *spec);
        sample_buffer.copy_interleaved_ref(decoded);

        Ok(Some(sample_buffer.samples().to_vec()))
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
