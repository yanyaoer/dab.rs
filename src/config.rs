use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub cache_dir: String,
    pub max_cache_size_mb: u64,
    pub audio_quality: AudioQuality,
    pub volume: f32,
    pub crossfade_ms: u32,
    // New cache-related settings
    pub cache_max_age_days: u32,
    pub cache_min_free_space_mb: u64,
    pub preload_next_tracks: u32,
    pub stream_url_expire_hours: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AudioQuality {
    Low,    // 96 kbps
    Medium, // 160 kbps
    High,   // 320 kbps
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cache_dir: dirs::cache_dir()
                .unwrap_or_else(|| std::env::temp_dir())
                .join("dab")
                .to_string_lossy()
                .to_string(),
            max_cache_size_mb: 1024, // 1GB
            audio_quality: AudioQuality::High,
            volume: 0.8,
            crossfade_ms: 0,
            // New cache settings with smart defaults
            cache_max_age_days: 30,       // Keep tracks for 30 days max
            cache_min_free_space_mb: 256, // Keep 256MB free space
            preload_next_tracks: 2,       // Preload next 2 tracks
            stream_url_expire_hours: 24,  // Stream URLs expire after 24 hours
        }
    }
}

impl Config {
    pub fn load() -> Self {
        // TODO: Load from config file
        Self::default()
    }
}
