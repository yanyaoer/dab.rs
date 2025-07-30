use std::sync::Arc;
use std::collections::VecDeque;
use serde::{Serialize, Deserialize};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u32,
    pub url: String,
    pub local_path: Option<String>,
    pub cover_url: Option<String>,
}

impl Track {
    pub fn from_url(url: &str) -> Self {
        Self {
            id: format!("{:x}", md5::compute(url)),
            title: Self::extract_title_from_url(url),
            artist: "Unknown Artist".to_string(),
            album: "Unknown Album".to_string(),
            duration_ms: 0,
            url: url.to_string(),
            local_path: None,
            cover_url: None,
        }
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
    repeat: Arc<RwLock<bool>>,
}

impl Queue {
    pub fn new() -> Self {
        Self {
            tracks: Arc::new(RwLock::new(VecDeque::new())),
            current_index: Arc::new(RwLock::new(None)),
            shuffle: Arc::new(RwLock::new(false)),
            repeat: Arc::new(RwLock::new(false)),
        }
    }
    
    pub async fn add_track(&self, track: Track) {
        let mut tracks = self.tracks.write().await;
        tracks.push_back(track);
    }
    
    pub async fn get_current_track(&self) -> Option<Track> {
        let tracks = self.tracks.read().await;
        let current_index = self.current_index.read().await;
        
        if let Some(index) = *current_index {
            tracks.get(index).cloned()
        } else {
            None
        }
    }
    
    pub async fn next_track(&self) -> Option<Track> {
        let tracks = self.tracks.read().await;
        let mut current_index = self.current_index.write().await;
        
        if tracks.is_empty() {
            return None;
        }
        
        let next_index = match *current_index {
            Some(index) => {
                if index + 1 < tracks.len() {
                    Some(index + 1)
                } else if *self.repeat.read().await {
                    Some(0)
                } else {
                    None
                }
            }
            None => Some(0),
        };
        
        *current_index = next_index;
        next_index.and_then(|i| tracks.get(i).cloned())
    }
    
    pub async fn previous_track(&self) -> Option<Track> {
        let tracks = self.tracks.read().await;
        let mut current_index = self.current_index.write().await;
        
        if tracks.is_empty() {
            return None;
        }
        
        let prev_index = match *current_index {
            Some(index) => {
                if index > 0 {
                    Some(index - 1)
                } else if *self.repeat.read().await {
                    Some(tracks.len() - 1)
                } else {
                    None
                }
            }
            None => Some(0),
        };
        
        *current_index = prev_index;
        prev_index.and_then(|i| tracks.get(i).cloned())
    }
    
    pub async fn len(&self) -> usize {
        let tracks = self.tracks.read().await;
        tracks.len()
    }
    
    pub async fn clear(&self) {
        let mut tracks = self.tracks.write().await;
        let mut current_index = self.current_index.write().await;
        tracks.clear();
        *current_index = None;
    }
}