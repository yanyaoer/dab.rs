use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, RwLock};
use log::{debug, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u32,
    pub local_path: Option<String>,
    pub cover_url: Option<String>,
    // Track metadata for online sources
    pub track_id: Option<String>,  // Original track ID from API
    pub artist_id: Option<String>,
    pub album_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StreamUrl {
    pub url: String,
    pub expires_at: u64, // Unix timestamp
}

impl StreamUrl {
    pub fn new(url: String, expires_in_seconds: Option<u64>) -> Self {
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() + expires_in_seconds.unwrap_or(3600); // Default 1 hour
        
        Self { url, expires_at }
    }
    
    pub fn is_expired(&self) -> bool {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() > self.expires_at
    }
    
    pub fn is_local(&self) -> bool {
        self.url.starts_with("file://") || self.url.starts_with("/")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RepeatMode {
    Off,
    One,
    All,
}

#[derive(Debug)]
pub enum QueueCommand {
    AddTrack(Track),
    AddNext(Track),
    AddBefore(usize, Track),
    RemoveTrack(usize),
    MoveTrack(usize, usize),
    Clear,
    Next,
    Previous,
    JumpTo(usize),
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    GetQueue(tokio::sync::oneshot::Sender<Vec<Track>>),
    GetCurrentIndex(tokio::sync::oneshot::Sender<Option<usize>>),
}

#[derive(Debug, Clone)]
pub enum QueueEvent {
    TrackAdded(Track, usize),
    TrackRemoved(usize),
    CurrentChanged(usize, Track),
    QueueCleared,
    ShuffleChanged(bool),
    RepeatChanged(RepeatMode),
    QueueUpdated(Vec<Track>),
}

impl Track {
    pub fn from_url(url: &str) -> Self {
        Self {
            id: format!("{:x}", md5::compute(url)),
            title: Self::extract_title_from_url(url),
            artist: "Unknown Artist".to_string(),
            album: "Unknown Album".to_string(),
            duration_ms: 0,
            local_path: if url.starts_with("file://") || url.starts_with("/") {
                Some(url.to_string())
            } else {
                None
            },
            cover_url: None,
            track_id: None,
            artist_id: None,
            album_id: None,
        }
    }
    
    pub fn from_dab_track(dab_track: &crate::search::DabTrack) -> Self {
        Self {
            id: dab_track.id.clone(),
            title: dab_track.title.clone(),
            artist: dab_track.artist.clone(),
            album: dab_track.album_title.as_ref().unwrap_or(&"Unknown Album".to_string()).clone(),
            duration_ms: dab_track.duration.map(|s| s * 1000).unwrap_or(0),
            local_path: None,
            cover_url: dab_track.album_cover.clone(),
            track_id: Some(dab_track.id.clone()),
            artist_id: dab_track.artist_id.clone(),
            album_id: dab_track.album_id.clone(),
        }
    }
    
    pub fn is_local(&self) -> bool {
        self.local_path.is_some()
    }
    
    pub fn requires_stream_url(&self) -> bool {
        !self.is_local() && self.track_id.is_some()
    }

    fn extract_title_from_url(url: &str) -> String {
        url.split('/')
            .last()
            .unwrap_or("Unknown")
            .split('.')
            .next()
            .unwrap_or("Unknown")
            .to_string()
    }
}

#[derive(Debug)]
pub struct Queue {
    tracks: Arc<RwLock<VecDeque<Track>>>,
    current_index: Arc<RwLock<Option<usize>>>,
    shuffle: Arc<RwLock<bool>>,
    repeat: Arc<RwLock<RepeatMode>>,
    
    // Communication channels
    command_tx: mpsc::UnboundedSender<QueueCommand>,
    event_tx: broadcast::Sender<QueueEvent>,
    _handle: tokio::task::JoinHandle<()>,
}

impl Queue {
    pub fn new() -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, _) = broadcast::channel(100);
        
        let tracks = Arc::new(RwLock::new(VecDeque::new()));
        let current_index = Arc::new(RwLock::new(None));
        let shuffle = Arc::new(RwLock::new(false));
        let repeat = Arc::new(RwLock::new(RepeatMode::Off));
        
        let handle = {
            let tracks = tracks.clone();
            let current_index = current_index.clone();
            let shuffle = shuffle.clone();
            let repeat = repeat.clone();
            let event_tx = event_tx.clone();
            
            tokio::spawn(async move {
                Self::run_queue_service(
                    command_rx,
                    event_tx,
                    tracks,
                    current_index,
                    shuffle,
                    repeat,
                ).await;
            })
        };
        
        Self {
            tracks,
            current_index,
            shuffle,
            repeat,
            command_tx,
            event_tx,
            _handle: handle,
        }
    }
    
    async fn run_queue_service(
        mut command_rx: mpsc::UnboundedReceiver<QueueCommand>,
        event_tx: broadcast::Sender<QueueEvent>,
        tracks: Arc<RwLock<VecDeque<Track>>>,
        current_index: Arc<RwLock<Option<usize>>>,
        shuffle: Arc<RwLock<bool>>,
        repeat: Arc<RwLock<RepeatMode>>,
    ) {
        info!("Queue service started");
        
        while let Some(command) = command_rx.recv().await {
            debug!("Handling queue command: {:?}", command);
            
            match command {
                QueueCommand::AddTrack(track) => {
                    let mut tracks_guard = tracks.write().await;
                    let index = tracks_guard.len();
                    tracks_guard.push_back(track.clone());
                    drop(tracks_guard);
                    
                    let _ = event_tx.send(QueueEvent::TrackAdded(track, index));
                    let _ = event_tx.send(QueueEvent::QueueUpdated(
                        tracks.read().await.iter().cloned().collect()
                    ));
                }
                
                QueueCommand::AddNext(track) => {
                    let mut tracks_guard = tracks.write().await;
                    let current_idx = *current_index.read().await;
                    let insert_index = if let Some(idx) = current_idx {
                        idx + 1
                    } else {
                        0
                    };
                    tracks_guard.insert(insert_index, track.clone());
                    drop(tracks_guard);
                    
                    let _ = event_tx.send(QueueEvent::TrackAdded(track, insert_index));
                    let _ = event_tx.send(QueueEvent::QueueUpdated(
                        tracks.read().await.iter().cloned().collect()
                    ));
                }
                
                QueueCommand::AddBefore(index, track) => {
                    let mut tracks_guard = tracks.write().await;
                    if index <= tracks_guard.len() {
                        tracks_guard.insert(index, track.clone());
                        drop(tracks_guard);
                        
                        let _ = event_tx.send(QueueEvent::TrackAdded(track, index));
                        let _ = event_tx.send(QueueEvent::QueueUpdated(
                            tracks.read().await.iter().cloned().collect()
                        ));
                    }
                }
                
                QueueCommand::RemoveTrack(index) => {
                    let mut tracks_guard = tracks.write().await;
                    if index < tracks_guard.len() {
                        tracks_guard.remove(index);
                        drop(tracks_guard);
                        
                        // Update current index if needed
                        let mut current_idx = current_index.write().await;
                        if let Some(current) = *current_idx {
                            if current == index {
                                *current_idx = None; // Current track was removed
                            } else if current > index {
                                *current_idx = Some(current - 1); // Adjust index
                            }
                        }
                        drop(current_idx);
                        
                        let _ = event_tx.send(QueueEvent::TrackRemoved(index));
                        let _ = event_tx.send(QueueEvent::QueueUpdated(
                            tracks.read().await.iter().cloned().collect()
                        ));
                    }
                }
                
                QueueCommand::MoveTrack(from_index, to_index) => {
                    let mut tracks_guard = tracks.write().await;
                    if from_index < tracks_guard.len() && to_index < tracks_guard.len() {
                        if let Some(track) = tracks_guard.remove(from_index) {
                            tracks_guard.insert(to_index, track);
                        }
                        drop(tracks_guard);
                        
                        let _ = event_tx.send(QueueEvent::QueueUpdated(
                            tracks.read().await.iter().cloned().collect()
                        ));
                    }
                }
                
                QueueCommand::Clear => {
                    tracks.write().await.clear();
                    *current_index.write().await = None;
                    
                    let _ = event_tx.send(QueueEvent::QueueCleared);
                    let _ = event_tx.send(QueueEvent::QueueUpdated(Vec::new()));
                }
                
                QueueCommand::Next => {
                    let tracks_guard = tracks.read().await;
                    let mut current_idx = current_index.write().await;
                    let repeat_mode = *repeat.read().await;
                    
                    if tracks_guard.is_empty() {
                        continue;
                    }
                    
                    let next_index = match *current_idx {
                        Some(idx) => {
                            if idx + 1 < tracks_guard.len() {
                                Some(idx + 1)
                            } else {
                                match repeat_mode {
                                    RepeatMode::All => Some(0),
                                    RepeatMode::One => Some(idx), // Stay on current
                                    RepeatMode::Off => None,
                                }
                            }
                        }
                        None => Some(0),
                    };
                    
                    *current_idx = next_index;
                    
                    if let Some(idx) = next_index {
                        if let Some(track) = tracks_guard.get(idx) {
                            let _ = event_tx.send(QueueEvent::CurrentChanged(idx, track.clone()));
                        }
                    }
                }
                
                QueueCommand::Previous => {
                    let tracks_guard = tracks.read().await;
                    let mut current_idx = current_index.write().await;
                    let repeat_mode = *repeat.read().await;
                    
                    if tracks_guard.is_empty() {
                        continue;
                    }
                    
                    let prev_index = match *current_idx {
                        Some(idx) => {
                            if idx > 0 {
                                Some(idx - 1)
                            } else {
                                match repeat_mode {
                                    RepeatMode::All => Some(tracks_guard.len() - 1),
                                    RepeatMode::One => Some(idx), // Stay on current
                                    RepeatMode::Off => None,
                                }
                            }
                        }
                        None => {
                            // If current is None, we're at the end. Go to the last track.
                            if tracks_guard.len() > 0 {
                                Some(tracks_guard.len() - 1)
                            } else {
                                None
                            }
                        }
                    };
                    
                    *current_idx = prev_index;
                    
                    if let Some(idx) = prev_index {
                        if let Some(track) = tracks_guard.get(idx) {
                            let _ = event_tx.send(QueueEvent::CurrentChanged(idx, track.clone()));
                        }
                    }
                }
                
                QueueCommand::JumpTo(index) => {
                    let tracks_guard = tracks.read().await;
                    if index < tracks_guard.len() {
                        *current_index.write().await = Some(index);
                        if let Some(track) = tracks_guard.get(index) {
                            let _ = event_tx.send(QueueEvent::CurrentChanged(index, track.clone()));
                        }
                    }
                }
                
                QueueCommand::SetShuffle(enabled) => {
                    *shuffle.write().await = enabled;
                    let _ = event_tx.send(QueueEvent::ShuffleChanged(enabled));
                }
                
                QueueCommand::SetRepeat(mode) => {
                    *repeat.write().await = mode;
                    let _ = event_tx.send(QueueEvent::RepeatChanged(mode));
                }
                
                QueueCommand::GetQueue(tx) => {
                    let queue_copy = tracks.read().await.iter().cloned().collect();
                    let _ = tx.send(queue_copy);
                }
                
                QueueCommand::GetCurrentIndex(tx) => {
                    let current = *current_index.read().await;
                    let _ = tx.send(current);
                }
            }
        }
        
        info!("Queue service stopped");
    }
    
    // Public interface methods using command channel
    pub async fn send_command(&self, command: QueueCommand) -> Result<(), String> {
        self.command_tx.send(command)
            .map_err(|_| "Failed to send queue command".to_string())
    }
    
    pub fn subscribe(&self) -> broadcast::Receiver<QueueEvent> {
        self.event_tx.subscribe()
    }
    
    // Convenience methods that use the command channel
    pub async fn add_track(&self, track: Track) {
        let _ = self.send_command(QueueCommand::AddTrack(track)).await;
    }

    pub async fn add_track_next(&self, track: Track) {
        let _ = self.send_command(QueueCommand::AddNext(track)).await;
    }

    pub async fn get_current_track(&self) -> Option<Track> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self.send_command(QueueCommand::GetCurrentIndex(tx)).await;
        
        if let Ok(Some(index)) = rx.await {
            let tracks = self.tracks.read().await;
            tracks.get(index).cloned()
        } else {
            None
        }
    }

    pub async fn next_track(&self) -> Option<Track> {
        let _ = self.send_command(QueueCommand::Next).await;
        // Small delay to allow command processing
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        self.get_current_track().await
    }

    pub async fn previous_track(&self) -> Option<Track> {
        let _ = self.send_command(QueueCommand::Previous).await;
        // Small delay to allow command processing
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        self.get_current_track().await
    }

    pub async fn len(&self) -> usize {
        let tracks = self.tracks.read().await;
        tracks.len()
    }

    pub async fn clear(&self) {
        let _ = self.send_command(QueueCommand::Clear).await;
    }
    
    pub async fn get_queue(&self) -> Vec<Track> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self.send_command(QueueCommand::GetQueue(tx)).await;
        rx.await.unwrap_or_default()
    }
    
    pub async fn get_current_index(&self) -> Option<usize> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self.send_command(QueueCommand::GetCurrentIndex(tx)).await;
        rx.await.unwrap_or(None)
    }
}
