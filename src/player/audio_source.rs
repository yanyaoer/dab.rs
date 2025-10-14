use log::{debug, error, trace};
use rodio::Source;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::decoder::AudioDecoder;
use crate::error::DabError;

pub struct DecodedAudioSource {
    decoder: Arc<Mutex<AudioDecoder>>,
    sample_rate: u32,
    channels: u16,
    current_frame: Vec<f32>,
    frame_position: usize,
    finished: bool,
    consecutive_underruns: u32,
}

impl DecodedAudioSource {
    pub fn new(decoder: AudioDecoder) -> Self {
        let sample_rate = decoder.sample_rate();
        let channels = decoder.channels();

        Self {
            decoder: Arc::new(Mutex::new(decoder)),
            sample_rate,
            channels,
            current_frame: Vec::new(),
            frame_position: 0,
            finished: false,
            consecutive_underruns: 0,
        }
    }

    fn load_next_frame(&mut self) -> bool {
        if self.finished {
            return false;
        }

        let mut decoder = match self.decoder.lock() {
            Ok(decoder) => decoder,
            Err(e) => {
                error!("Failed to lock decoder: {}", e);
                self.finished = true;
                return false;
            }
        };

        match decoder.next_frame() {
            Ok(Some(samples)) => {
                self.current_frame = samples;
                self.frame_position = 0;
                self.consecutive_underruns = 0; // Reset counter on successful read
                true
            }
            Ok(None) => {
                debug!("Audio decoder reached end of stream");
                self.finished = true;
                false
            }
            Err(DabError::TemporaryBufferUnderrun) => {
                // This is a temporary condition during streaming
                self.consecutive_underruns += 1;

                if self.consecutive_underruns > 100 {
                    // If we've had too many consecutive underruns, something is wrong
                    error!("Too many consecutive buffer underruns ({}), stopping playback", self.consecutive_underruns);
                    self.finished = true;
                    return false;
                }

                trace!("Buffer underrun #{}, inserting silence to maintain audio continuity", self.consecutive_underruns);

                // Instead of stopping playback, insert a small amount of silence
                // This prevents pops but maintains timing
                let silence_samples = (self.sample_rate as usize * self.channels as usize) / 100; // 10ms of silence
                self.current_frame = vec![0.0; silence_samples];
                self.frame_position = 0;

                // Sleep briefly to give the buffer time to fill
                std::thread::sleep(Duration::from_millis(5));

                true // Continue playback with silence
            }
            Err(e) => {
                error!("Failed to decode audio frame: {}", e);
                self.finished = true;
                false
            }
        }
    }
}

impl Iterator for DecodedAudioSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        // If we've consumed all samples in current frame, load next frame
        if self.frame_position >= self.current_frame.len() {
            if !self.load_next_frame() {
                return None;
            }
        }

        if self.frame_position < self.current_frame.len() {
            let sample = self.current_frame[self.frame_position];
            self.frame_position += 1;
            Some(sample)
        } else {
            None
        }
    }
}

impl Source for DecodedAudioSource {
    fn current_frame_len(&self) -> Option<usize> {
        if self.finished {
            None
        } else {
            Some(self.current_frame.len() - self.frame_position)
        }
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        // Could implement this by reading duration from decoder metadata
        None
    }
}
