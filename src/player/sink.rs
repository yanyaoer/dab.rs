use log::{debug, info};
use rodio::{OutputStream, OutputStreamHandle, Sink};
use std::sync::{Arc, Mutex};

use super::audio_source::DecodedAudioSource;
use super::decoder::AudioDecoder;
use crate::error::{DabError, DabResult};

// Wrapper to make AudioSink Send + Sync by using Send/Sync types internally
pub struct AudioSink {
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    sink: Arc<Mutex<Option<Sink>>>,
    volume: Arc<Mutex<f32>>,
}

// Manually implement Send and Sync since rodio types don't implement them by default
// This is safe as long as we only access them through Arc<Mutex<>>
unsafe impl Send for AudioSink {}
unsafe impl Sync for AudioSink {}

impl AudioSink {
    pub fn new() -> DabResult<Self> {
        let (_stream, stream_handle) = OutputStream::try_default()
            .map_err(|e| DabError::Audio(format!("Failed to create audio stream: {}", e)))?;

        info!("Audio sink initialized");

        Ok(Self {
            _stream,
            stream_handle,
            sink: Arc::new(Mutex::new(None)),
            volume: Arc::new(Mutex::new(0.8)),
        })
    }

    pub fn play(&mut self, decoder: AudioDecoder, volume: f32) -> DabResult<()> {
        // Create new sink
        let sink = Sink::try_new(&self.stream_handle)
            .map_err(|e| DabError::Audio(format!("Failed to create sink: {}", e)))?;

        sink.set_volume(volume);

        // Create audio source from decoder
        let audio_source = DecodedAudioSource::new(decoder);

        // Add the source to the sink
        sink.append(audio_source);

        sink.play();

        // Store sink
        *self.sink.lock().unwrap() = Some(sink);
        *self.volume.lock().unwrap() = volume;

        info!("Started playback with audio decoder");
        Ok(())
    }

    pub fn pause(&self) -> DabResult<()> {
        if let Some(ref sink) = *self.sink.lock().unwrap() {
            sink.pause();
            info!("Playback paused");
        }
        Ok(())
    }

    pub fn resume(&self) -> DabResult<()> {
        if let Some(ref sink) = *self.sink.lock().unwrap() {
            sink.play();
            info!("Playback resumed");
        }
        Ok(())
    }

    pub fn stop(&self) -> DabResult<()> {
        if let Some(sink) = self.sink.lock().unwrap().take() {
            sink.stop();
            info!("Playback stopped");
        }
        Ok(())
    }

    pub fn set_volume(&self, volume: f32) -> DabResult<()> {
        let clamped_volume = volume.clamp(0.0, 1.0);

        if let Some(ref sink) = *self.sink.lock().unwrap() {
            sink.set_volume(clamped_volume);
        }

        *self.volume.lock().unwrap() = clamped_volume;
        debug!("Volume set to: {:.2}", clamped_volume);
        Ok(())
    }

    pub fn is_paused(&self) -> bool {
        if let Some(ref sink) = *self.sink.lock().unwrap() {
            sink.is_paused()
        } else {
            true
        }
    }

    pub fn is_empty(&self) -> bool {
        if let Some(ref sink) = *self.sink.lock().unwrap() {
            sink.empty()
        } else {
            true
        }
    }
}
