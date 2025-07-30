use std::sync::{Arc, Mutex};
use std::time::Duration;
use rodio::Source;
use log::{debug, warn, error};

use crate::error::{DabResult, DabError};
use super::decoder::AudioDecoder;

pub struct DecodedAudioSource {
    decoder: Arc<Mutex<AudioDecoder>>,
    sample_rate: u32,
    channels: u16,
    current_frame: Vec<f32>,
    frame_position: usize,
    finished: bool,
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
                true
            }
            Ok(None) => {
                debug!("Audio decoder reached end of stream");
                self.finished = true;
                false
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