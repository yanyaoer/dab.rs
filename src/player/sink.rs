use log::{debug, info, warn};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::io::{BufReader, Cursor};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::audio_source::DecodedAudioSource;
use super::decoder::AudioDecoder;
use crate::error::{DabError, DabResult};

// Wrapper to make AudioSink Send + Sync by using Send/Sync types internally
pub struct AudioSink {
    stream: Option<OutputStream>,
    stream_handle: Option<OutputStreamHandle>,
    audio_disabled: bool,
    sink: Arc<Mutex<Option<Sink>>>,
    volume: Arc<Mutex<f32>>,
    // Position tracking
    start_time: Arc<Mutex<Option<Instant>>>,
    paused_duration: Arc<Mutex<Duration>>,
    last_pause_time: Arc<Mutex<Option<Instant>>>,
}

// Manually implement Send and Sync since rodio types don't implement them by default
// This is safe as long as we only access them through Arc<Mutex<>>
unsafe impl Send for AudioSink {}
unsafe impl Sync for AudioSink {}

impl AudioSink {
    pub fn new() -> DabResult<Self> {
        let (stream, stream_handle, audio_disabled) = match OutputStream::try_default() {
            Ok((stream, handle)) => {
                info!("Audio sink initialized");
                (Some(stream), Some(handle), false)
            }
            Err(e) => {
                warn!(
                    "Audio output not available: {}. Continuing in silent mode",
                    e
                );
                (None, None, true)
            }
        };

        Ok(Self {
            stream,
            stream_handle,
            audio_disabled,
            sink: Arc::new(Mutex::new(None)),
            volume: Arc::new(Mutex::new(0.8)),
            start_time: Arc::new(Mutex::new(None)),
            paused_duration: Arc::new(Mutex::new(Duration::ZERO)),
            last_pause_time: Arc::new(Mutex::new(None)),
        })
    }

    pub fn play(&mut self, decoder: AudioDecoder, volume: f32) -> DabResult<()> {
        if self.audio_disabled {
            info!("Audio disabled; skipping playback");
            *self.start_time.lock().unwrap() = Some(Instant::now());
            *self.paused_duration.lock().unwrap() = Duration::ZERO;
            *self.last_pause_time.lock().unwrap() = None;
            return Ok(());
        }

        // Create new sink
        let sink = Sink::try_new(self.stream_handle.as_ref().unwrap())
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

        // Reset position tracking for new track
        *self.start_time.lock().unwrap() = Some(Instant::now());
        *self.paused_duration.lock().unwrap() = Duration::ZERO;
        *self.last_pause_time.lock().unwrap() = None;

        info!("Started playback with audio decoder");
        Ok(())
    }

    /// Play audio directly from memory using rodio (like test_squid_direct)
    pub fn play_from_memory(&mut self, audio_data: Vec<u8>, volume: f32) -> DabResult<()> {
        if self.audio_disabled {
            info!("Audio disabled; skipping playback");
            *self.start_time.lock().unwrap() = Some(Instant::now());
            *self.paused_duration.lock().unwrap() = Duration::ZERO;
            *self.last_pause_time.lock().unwrap() = None;
            return Ok(());
        }

        info!(
            "Playing audio from memory buffer ({} bytes) using rodio directly",
            audio_data.len()
        );

        // Create new sink
        let sink = Sink::try_new(self.stream_handle.as_ref().unwrap())
            .map_err(|e| DabError::Audio(format!("Failed to create sink: {}", e)))?;

        sink.set_volume(volume);

        // Create decoder directly from memory (like test_squid_direct)
        let cursor = Cursor::new(audio_data);
        let buf_reader = BufReader::new(cursor);

        let source = Decoder::new(buf_reader)
            .map_err(|e| DabError::Audio(format!("Failed to create rodio decoder: {}", e)))?;

        // Add the source to the sink and play
        sink.append(source);
        sink.play();

        // Store sink
        *self.sink.lock().unwrap() = Some(sink);
        *self.volume.lock().unwrap() = volume;

        // Reset position tracking for new track
        *self.start_time.lock().unwrap() = Some(Instant::now());
        *self.paused_duration.lock().unwrap() = Duration::ZERO;
        *self.last_pause_time.lock().unwrap() = None;

        info!("Started playback with rodio decoder (bypassing symphonia)");
        Ok(())
    }

    pub fn pause(&self) -> DabResult<()> {
        if self.audio_disabled {
            return Ok(());
        }

        if let Some(ref sink) = *self.sink.lock().unwrap() {
            sink.pause();
            // Record pause time for position tracking
            *self.last_pause_time.lock().unwrap() = Some(Instant::now());
            info!("Playback paused");
        }
        Ok(())
    }

    pub fn resume(&self) -> DabResult<()> {
        if self.audio_disabled {
            return Ok(());
        }

        if let Some(ref sink) = *self.sink.lock().unwrap() {
            sink.play();
            // Update paused duration when resuming
            if let Some(pause_time) = self.last_pause_time.lock().unwrap().take() {
                let pause_duration = pause_time.elapsed();
                *self.paused_duration.lock().unwrap() += pause_duration;
            }
            info!("Playback resumed");
        }
        Ok(())
    }

    pub fn stop(&self) -> DabResult<()> {
        if self.audio_disabled {
            *self.start_time.lock().unwrap() = None;
            *self.paused_duration.lock().unwrap() = Duration::ZERO;
            *self.last_pause_time.lock().unwrap() = None;
            return Ok(());
        }

        if let Some(sink) = self.sink.lock().unwrap().take() {
            sink.stop();
            // Reset position tracking
            *self.start_time.lock().unwrap() = None;
            *self.paused_duration.lock().unwrap() = Duration::ZERO;
            *self.last_pause_time.lock().unwrap() = None;
            info!("Playback stopped");
        }
        Ok(())
    }

    pub fn set_volume(&self, volume: f32) -> DabResult<()> {
        let clamped_volume = volume.clamp(0.0, 1.0);

        if self.audio_disabled {
            *self.volume.lock().unwrap() = clamped_volume;
            return Ok(());
        }

        if let Some(ref sink) = *self.sink.lock().unwrap() {
            sink.set_volume(clamped_volume);
        }

        *self.volume.lock().unwrap() = clamped_volume;
        debug!("Volume set to: {:.2}", clamped_volume);
        Ok(())
    }

    pub fn is_paused(&self) -> bool {
        if self.audio_disabled {
            return true;
        }

        if let Some(ref sink) = *self.sink.lock().unwrap() {
            sink.is_paused()
        } else {
            true
        }
    }

    pub fn is_empty(&self) -> bool {
        if self.audio_disabled {
            return true;
        }

        if let Some(ref sink) = *self.sink.lock().unwrap() {
            sink.empty()
        } else {
            true
        }
    }

    /// Get current playback position in milliseconds
    pub fn get_position_ms(&self) -> u32 {
        let start_time_guard = self.start_time.lock().unwrap();
        let paused_duration_guard = self.paused_duration.lock().unwrap();
        let last_pause_time_guard = self.last_pause_time.lock().unwrap();

        if self.audio_disabled {
            return 0;
        }

        if let Some(start_time) = *start_time_guard {
            let total_elapsed = start_time.elapsed();
            let current_paused_duration = if let Some(pause_time) = *last_pause_time_guard {
                // Currently paused, add current pause duration
                *paused_duration_guard + pause_time.elapsed()
            } else {
                // Not currently paused
                *paused_duration_guard
            };

            // Subtract paused time from total elapsed time
            let playback_position = total_elapsed.saturating_sub(current_paused_duration);
            playback_position.as_millis() as u32
        } else {
            0
        }
    }
}
