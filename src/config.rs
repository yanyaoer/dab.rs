use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

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
    // API configuration
    pub api: ApiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    pub base_url: String,
    pub timeout_seconds: u64,
    pub max_retries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AudioQuality {
    Low,    // 96 kbps
    Medium, // 160 kbps
    High,   // 320 kbps
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            base_url: "https://dab.yeet.su/api".to_string(),
            timeout_seconds: 30,
            max_retries: 3,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cache_dir: dirs::cache_dir()
                .unwrap_or_else(|| std::env::temp_dir())
                .join("dab")
                .to_string_lossy()
                .to_string(),
            max_cache_size_mb: 1024 * 5, // 5GB
            audio_quality: AudioQuality::High,
            volume: 0.8,
            crossfade_ms: 0,
            // New cache settings with smart defaults
            cache_max_age_days: 30,       // Keep tracks for 30 days max
            cache_min_free_space_mb: 256, // Keep 256MB free space
            preload_next_tracks: 2,       // Preload next 2 tracks
            stream_url_expire_hours: 24,  // Stream URLs expire after 24 hours
            api: ApiConfig::default(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let config_path = Self::config_path();

        // Try to load from file if it exists
        if config_path.exists() {
            match fs::read_to_string(&config_path) {
                Ok(contents) => {
                    match toml::from_str(&contents) {
                        Ok(config) => {
                            log::info!("Config loaded from {:?}", config_path);
                            return config;
                        }
                        Err(e) => {
                            log::warn!("Failed to parse config file: {}", e);
                            log::warn!("Using default config");
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Failed to read config file: {}", e);
                    log::warn!("Using default config");
                }
            }
        } else {
            // Create default config file
            let default_config = Self::default();
            if let Err(e) = default_config.save() {
                log::warn!("Failed to save default config: {}", e);
            } else {
                log::info!("Default config created at {:?}", config_path);
            }
            return default_config;
        }

        Self::default()
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let config_path = Self::config_path();

        // Ensure config directory exists
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let toml_string = toml::to_string_pretty(self)?;
        fs::write(config_path, toml_string)?;
        Ok(())
    }

    fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("dab")
            .join("config.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.api.base_url, "https://dab.yeet.su/api");
        assert_eq!(config.api.timeout_seconds, 30);
        assert_eq!(config.api.max_retries, 3);
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::default();
        let toml_str = toml::to_string_pretty(&config).expect("Failed to serialize config");

        // Check that important fields are present
        assert!(toml_str.contains("cache_dir"));
        assert!(toml_str.contains("[api]"));
        assert!(toml_str.contains("base_url"));
    }

    #[test]
    fn test_api_config_custom_url() {
        let api_config = ApiConfig {
            base_url: "https://custom.api.server/v1".to_string(),
            timeout_seconds: 45,
            max_retries: 5,
        };

        assert_eq!(api_config.base_url, "https://custom.api.server/v1");
        assert_eq!(api_config.timeout_seconds, 45);
        assert_eq!(api_config.max_retries, 5);
    }
}
