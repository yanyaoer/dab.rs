use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};

mod audio_source;
mod decoder;
mod engine;
mod loader;
mod queue;
mod sink;

pub use engine::PlayerEngine;
pub use queue::{Queue, Track};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerStatus {
    pub state: PlayerState,
    pub current_track: Option<Track>,
    pub position_ms: u32,
    pub duration_ms: u32,
    pub volume: f32,
    pub queue_length: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PlayerState {
    Stopped,
    Loading,
    Playing,
    Paused,
    Buffering,
}

#[derive(Debug)]
pub enum PlayerCommand {
    LoadAndPlay(String),
    Play,
    Pause,
    Resume,
    Stop,
    Next,
    Previous,
    Seek(u32),
    AddToQueue(String),
    AddNext(String),
    ClearAndPlay(Vec<String>),
    SetVolume(f32),
    GetStatus(tokio::sync::oneshot::Sender<PlayerStatus>),
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    StateChanged(PlayerState),
    TrackChanged(Track),
    PositionChanged(u32),
    VolumeChanged(f32),
    QueueChanged,
    Error(String),
}
