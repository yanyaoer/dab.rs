use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub cache_dir: String,
    pub max_cache_size_mb: u64,
    pub audio_quality: AudioQuality,
    pub volume: f32,
    pub crossfade_ms: u32,
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
        }
    }
}

impl Config {
    pub fn load() -> Self {
        // TODO: Load from config file
        Self::default()
    }
}
