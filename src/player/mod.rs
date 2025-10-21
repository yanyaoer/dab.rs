use serde::{Deserialize, Serialize};

mod audio_source;
mod decoder;
mod download_manager;
mod engine;
mod loader;
mod preloader;
mod queue;
mod sink;
mod streaming;

pub use engine::PlayerEngine;
pub use queue::{Queue, RepeatMode, Track};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerStatus {
    pub state: PlayerState,
    pub current_track: Option<Track>,
    pub position_ms: u32,
    pub duration_ms: u32,
    pub volume: f32,
    pub queue_length: usize,
    pub repeat_mode: RepeatMode,
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
    // Simplified commands - use Track for all operations
    LoadAndPlayTrack(Track),
    Play,
    Pause,
    Resume,
    Stop,
    Next,
    Previous,
    Seek(u32),
    AddTrackToQueue(Track),
    AddTrackNext(Track),
    ClearAndPlayTracks(Vec<Track>),
    SetVolume(f32),
    SetRepeatMode(RepeatMode),
    GetStatus(tokio::sync::oneshot::Sender<PlayerStatus>),
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    StateChanged(PlayerState),
    TrackChanged(Track),
    PositionChanged(u32),
    VolumeChanged(f32),
    QueueChanged,
    RepeatModeChanged(RepeatMode),
    TrackEnded,
    Error(String),
    // New streaming-related events
    DownloadProgress { track_id: String, progress: f32 },
    StreamReady { track_id: String },
    BufferingStart { track_id: String },
    BufferingEnd { track_id: String },
    DownloadStarted { track_id: String },
    DownloadCompleted { track_id: String },
    DownloadFailed { track_id: String, error: String },
}
