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
    // Streaming buffer settings (0 = download complete file before playback)
    pub streaming_buffer: u32,
    // API configuration
    pub api: ApiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    pub backend: BackendType,
    pub targets: Vec<ApiTarget>,
    pub timeout_seconds: u64,
    pub max_retries: u32,
    pub use_proxy: bool,
    pub proxy_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BackendType {
    Squid, // DAB.rs backend (only supported backend)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiTarget {
    pub name: String,
    pub base_url: String,
    pub weight: u32,
    pub requires_proxy: bool,
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
            backend: BackendType::Squid, // Use Squid as default backend
            targets: vec![
                // Primary endpoints with highest weight
                ApiTarget {
                    name: "kraken-primary".to_string(),
                    base_url: "https://kraken.squid.wtf".to_string(),
                    weight: 25,
                    requires_proxy: false,
                },
                ApiTarget {
                    name: "triton-secondary".to_string(),
                    base_url: "https://triton.squid.wtf".to_string(),
                    weight: 25,
                    requires_proxy: false,
                },
                // Tertiary endpoints
                ApiTarget {
                    name: "zeus-tertiary".to_string(),
                    base_url: "https://zeus.squid.wtf".to_string(),
                    weight: 20,
                    requires_proxy: false,
                },
                ApiTarget {
                    name: "aether-quaternary".to_string(),
                    base_url: "https://aether.squid.wtf".to_string(),
                    weight: 20,
                    requires_proxy: false,
                },
                // Backup endpoints with lower weight
                ApiTarget {
                    name: "vercel-fastapi".to_string(),
                    base_url: "https://tidal-api-2.binimum.org".to_string(),
                    weight: 5,
                    requires_proxy: false,
                },
                ApiTarget {
                    name: "proxied-primary".to_string(),
                    base_url: "https://tidal.401658.xyz".to_string(),
                    weight: 5,
                    requires_proxy: false,
                },
            ],
            timeout_seconds: 30,
            max_retries: 3,
            use_proxy: false,
            proxy_url: None,
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
            streaming_buffer: 0,          // 0 = download complete before playback (recommended)
            api: ApiConfig::default(),
        }
    }
}

impl ApiConfig {
    /// Select an API target based on weight distribution
    pub fn select_target(&self) -> &ApiTarget {
        if self.targets.is_empty() {
            panic!("No API targets configured");
        }

        // Calculate cumulative weights
        let total_weight: u32 = self.targets.iter().map(|t| t.weight).sum();
        if total_weight == 0 {
            return &self.targets[0];
        }

        // Random selection based on weight
        let random_value = rand::random::<f32>() * total_weight as f32;
        let mut cumulative = 0u32;

        for target in &self.targets {
            cumulative += target.weight;
            if random_value < cumulative as f32 {
                return target;
            }
        }

        &self.targets[0]
    }

    /// Get the primary (highest weighted) target
    pub fn primary_target(&self) -> &ApiTarget {
        self.targets
            .iter()
            .max_by_key(|t| t.weight)
            .unwrap_or(&self.targets[0])
    }

    /// Get all targets ordered by weight (descending)
    pub fn targets_by_weight(&self) -> Vec<&ApiTarget> {
        let mut targets: Vec<&ApiTarget> = self.targets.iter().collect();
        targets.sort_by(|a, b| b.weight.cmp(&a.weight));
        targets
    }
}

impl Config {
    pub fn load() -> Self {
        let config_path = Self::config_path();

        // Try to load from file if it exists
        let mut config = if config_path.exists() {
            match fs::read_to_string(&config_path) {
                Ok(contents) => match toml::from_str(&contents) {
                    Ok(config) => {
                        log::info!("Config loaded from {:?}", config_path);
                        config
                    }
                    Err(e) => {
                        log::warn!("Failed to parse config file: {}", e);
                        log::warn!("Using default config");
                        Self::default()
                    }
                },
                Err(e) => {
                    log::warn!("Failed to read config file: {}", e);
                    log::warn!("Using default config");
                    Self::default()
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
            default_config
        };

        // Override with environment variables
        config.apply_env_overrides();

        config
    }

    /// Apply environment variable overrides to the configuration
    /// Environment variables should be prefixed with DAB_
    /// Examples:
    ///   DAB_STREAMING_BUFFER=0
    ///   DAB_AUDIO_QUALITY=High
    ///   DAB_VOLUME=0.8
    ///   DAB_MAX_CACHE_SIZE_MB=1024
    fn apply_env_overrides(&mut self) {
        use std::env;

        // Check for streaming_buffer override
        if let Ok(val) = env::var("DAB_STREAMING_BUFFER") {
            if let Ok(buffer_mb) = val.parse::<u32>() {
                log::info!("Overriding streaming_buffer from env: {} MB", buffer_mb);
                self.streaming_buffer = buffer_mb;
            }
        }

        // Check for audio_quality override
        if let Ok(val) = env::var("DAB_AUDIO_QUALITY") {
            let quality = match val.to_lowercase().as_str() {
                "low" => AudioQuality::Low,
                "medium" => AudioQuality::Medium,
                "high" => AudioQuality::High,
                _ => {
                    log::warn!(
                        "Invalid audio quality in env: {}. Using existing value.",
                        val
                    );
                    self.audio_quality.clone()
                }
            };
            if val.to_lowercase() == "low"
                || val.to_lowercase() == "medium"
                || val.to_lowercase() == "high"
            {
                log::info!("Overriding audio_quality from env: {:?}", quality);
                self.audio_quality = quality;
            }
        }

        // Check for volume override
        if let Ok(val) = env::var("DAB_VOLUME") {
            if let Ok(volume) = val.parse::<f32>() {
                if volume >= 0.0 && volume <= 1.0 {
                    log::info!("Overriding volume from env: {}", volume);
                    self.volume = volume;
                } else {
                    log::warn!(
                        "Invalid volume in env: {}. Must be between 0.0 and 1.0",
                        volume
                    );
                }
            }
        }

        // Check for max_cache_size_mb override
        if let Ok(val) = env::var("DAB_MAX_CACHE_SIZE_MB") {
            if let Ok(size_mb) = val.parse::<u64>() {
                log::info!("Overriding max_cache_size_mb from env: {} MB", size_mb);
                self.max_cache_size_mb = size_mb;
            }
        }

        // Check for cache_dir override
        if let Ok(val) = env::var("DAB_CACHE_DIR") {
            if !val.is_empty() {
                log::info!("Overriding cache_dir from env: {}", val);
                self.cache_dir = val;
            }
        }

        // Check for preload_next_tracks override
        if let Ok(val) = env::var("DAB_PRELOAD_NEXT_TRACKS") {
            if let Ok(tracks) = val.parse::<u32>() {
                log::info!("Overriding preload_next_tracks from env: {}", tracks);
                self.preload_next_tracks = tracks;
            }
        }

        // Check for crossfade_ms override
        if let Ok(val) = env::var("DAB_CROSSFADE_MS") {
            if let Ok(ms) = val.parse::<u32>() {
                log::info!("Overriding crossfade_ms from env: {} ms", ms);
                self.crossfade_ms = ms;
            }
        }

        // Check for API backend override
        if let Ok(val) = env::var("DAB_API_BACKEND") {
            if val.to_lowercase() != "squid" {
                log::warn!(
                    "Invalid API backend in env: {}. Only 'squid' is supported.",
                    val
                );
            }
            // Backend is always Squid now, no need to change
        }

        // Check for API timeout override
        if let Ok(val) = env::var("DAB_API_TIMEOUT_SECONDS") {
            if let Ok(timeout) = val.parse::<u64>() {
                log::info!("Overriding API timeout from env: {} seconds", timeout);
                self.api.timeout_seconds = timeout;
            }
        }
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
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("dab")
            .join("config.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.api.backend as u32, BackendType::Squid as u32);
        assert!(!config.api.targets.is_empty());
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
        assert!(toml_str.contains("backend"));
        assert!(toml_str.contains("[[api.targets]]"));
    }

    #[test]
    fn test_api_config_custom_url() {
        let api_config = ApiConfig {
            backend: BackendType::Squid,
            targets: vec![ApiTarget {
                name: "custom-squid".to_string(),
                base_url: "https://custom.api.server/v1".to_string(),
                weight: 100,
                requires_proxy: false,
            }],
            timeout_seconds: 45,
            max_retries: 5,
            use_proxy: true,
            proxy_url: Some("/api/proxy".to_string()),
        };

        assert_eq!(
            api_config.targets[0].base_url,
            "https://custom.api.server/v1"
        );
        assert_eq!(api_config.timeout_seconds, 45);
        assert_eq!(api_config.max_retries, 5);
        assert!(api_config.use_proxy);
    }

    #[test]
    fn test_api_target_selection() {
        let api_config = ApiConfig {
            backend: BackendType::Squid,
            targets: vec![
                ApiTarget {
                    name: "high-weight".to_string(),
                    base_url: "https://high.server".to_string(),
                    weight: 90,
                    requires_proxy: false,
                },
                ApiTarget {
                    name: "low-weight".to_string(),
                    base_url: "https://low.server".to_string(),
                    weight: 10,
                    requires_proxy: false,
                },
            ],
            timeout_seconds: 30,
            max_retries: 3,
            use_proxy: false,
            proxy_url: None,
        };

        // Primary target should be the highest weighted
        assert_eq!(api_config.primary_target().name, "high-weight");

        // Test weight ordering
        let ordered = api_config.targets_by_weight();
        assert_eq!(ordered[0].name, "high-weight");
        assert_eq!(ordered[1].name, "low-weight");
    }
}
