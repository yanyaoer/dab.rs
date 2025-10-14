use dab::{Config, DabError};
use std::env;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

#[tokio::test]
async fn test_config_default_creation() {
    let config = Config::default();

    // Test default values
    assert!(!config.api_base_url.is_empty());
    assert!(config.cache_size_mb > 0);
    assert!(config.download_threads > 0);
    assert!(config.max_concurrent_downloads > 0);
    assert!((0.0..=1.0).contains(&config.default_volume));
}

#[tokio::test]
async fn test_config_file_loading() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    // Create a test config file
    let config_content = r#"
api_base_url = "https://test-api.example.com"
cache_size_mb = 512
download_threads = 8
max_concurrent_downloads = 6
default_volume = 0.8
enable_notifications = false
enable_discord_rpc = true
log_level = "debug"
cache_directory = "/tmp/test-cache"
log_file = "/tmp/test.log"

[ui]
show_cover_art = false
color_scheme = "dark"
compact_mode = true

[network]
timeout_seconds = 45
retry_attempts = 5
user_agent = "DabTestClient/1.0"

[audio]
buffer_size = 4096
sample_rate = 48000
channels = 2
bit_depth = 24
"#;

    fs::write(&config_path, config_content).unwrap();

    // Load config from file
    let config = Config::load_from_file(&config_path).unwrap();

    assert_eq!(config.api_base_url, "https://test-api.example.com");
    assert_eq!(config.cache_size_mb, 512);
    assert_eq!(config.download_threads, 8);
    assert_eq!(config.max_concurrent_downloads, 6);
    assert_eq!(config.default_volume, 0.8);
    assert!(!config.enable_notifications);
    assert!(config.enable_discord_rpc);
    assert_eq!(config.log_level, "debug");
    assert_eq!(config.cache_directory, Some("/tmp/test-cache".to_string()));
    assert_eq!(config.log_file, Some("/tmp/test.log".to_string()));
}

#[tokio::test]
async fn test_config_partial_file_loading() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("partial_config.toml");

    // Create a partial config file (only some values)
    let config_content = r#"
api_base_url = "https://partial-api.example.com"
cache_size_mb = 256
"#;

    fs::write(&config_path, config_content).unwrap();

    // Load config from file (should use defaults for missing values)
    let config = Config::load_from_file(&config_path).unwrap();

    assert_eq!(config.api_base_url, "https://partial-api.example.com");
    assert_eq!(config.cache_size_mb, 256);
    // These should be defaults since they weren't specified
    assert!(config.download_threads > 0);
    assert!(config.max_concurrent_downloads > 0);
}

#[tokio::test]
async fn test_config_invalid_file() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("invalid_config.toml");

    // Create an invalid TOML file
    let invalid_content = r#"
this is not valid toml
[unclosed section
invalid = 
"#;

    fs::write(&config_path, invalid_content).unwrap();

    // Loading should fail gracefully
    let result = Config::load_from_file(&config_path);
    assert!(result.is_err());

    if let Err(e) = result {
        assert!(matches!(e, DabError::ConfigError(_)));
    }
}

#[tokio::test]
async fn test_config_nonexistent_file() {
    let nonexistent_path = PathBuf::from("/this/path/does/not/exist/config.toml");

    // Loading nonexistent file should fail gracefully
    let result = Config::load_from_file(&nonexistent_path);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_config_save_to_file() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("save_test.toml");

    // Create a config with custom values
    let mut config = Config::default();
    config.api_base_url = "https://save-test.example.com".to_string();
    config.cache_size_mb = 1024;
    config.default_volume = 0.75;

    // Save to file
    config.save_to_file(&config_path).unwrap();

    // Verify file was created
    assert!(config_path.exists());

    // Load it back and verify
    let loaded_config = Config::load_from_file(&config_path).unwrap();
    assert_eq!(loaded_config.api_base_url, config.api_base_url);
    assert_eq!(loaded_config.cache_size_mb, config.cache_size_mb);
    assert_eq!(loaded_config.default_volume, config.default_volume);
}

#[tokio::test]
async fn test_config_environment_variables() {
    // Set environment variables
    env::set_var("DAB_API_BASE_URL", "https://env-api.example.com");
    env::set_var("DAB_CACHE_SIZE_MB", "2048");
    env::set_var("DAB_DEFAULT_VOLUME", "0.9");
    env::set_var("DAB_LOG_LEVEL", "trace");

    // Load config with environment variable overrides
    let config = Config::load_with_env_overrides().unwrap();

    assert_eq!(config.api_base_url, "https://env-api.example.com");
    assert_eq!(config.cache_size_mb, 2048);
    assert_eq!(config.default_volume, 0.9);
    assert_eq!(config.log_level, "trace");

    // Clean up environment variables
    env::remove_var("DAB_API_BASE_URL");
    env::remove_var("DAB_CACHE_SIZE_MB");
    env::remove_var("DAB_DEFAULT_VOLUME");
    env::remove_var("DAB_LOG_LEVEL");
}

#[tokio::test]
async fn test_config_validation() {
    let mut config = Config::default();

    // Test valid config
    assert!(config.validate().is_ok());

    // Test invalid cache size
    config.cache_size_mb = 0;
    assert!(config.validate().is_err());

    config.cache_size_mb = 100; // Reset to valid value

    // Test invalid volume
    config.default_volume = 1.5; // Above 1.0
    assert!(config.validate().is_err());

    config.default_volume = -0.1; // Below 0.0
    assert!(config.validate().is_err());

    config.default_volume = 0.5; // Reset to valid value

    // Test invalid thread count
    config.download_threads = 0;
    assert!(config.validate().is_err());

    config.download_threads = 4; // Reset to valid value

    // Test invalid concurrent downloads
    config.max_concurrent_downloads = 0;
    assert!(config.validate().is_err());

    // After fixing all issues, should be valid
    config.max_concurrent_downloads = 3;
    assert!(config.validate().is_ok());
}

#[tokio::test]
async fn test_config_merge() {
    let mut base_config = Config::default();
    base_config.api_base_url = "https://base.example.com".to_string();
    base_config.cache_size_mb = 512;

    let mut override_config = Config::default();
    override_config.api_base_url = "https://override.example.com".to_string();
    override_config.default_volume = 0.8;

    // Merge configs
    let merged = base_config.merge(override_config);

    // Override values should take precedence
    assert_eq!(merged.api_base_url, "https://override.example.com");
    assert_eq!(merged.default_volume, 0.8);

    // Base values should remain where not overridden
    assert_eq!(merged.cache_size_mb, 512);
}

#[tokio::test]
async fn test_config_directory_creation() {
    let temp_dir = TempDir::new().unwrap();
    let cache_dir = temp_dir.path().join("cache");
    let log_dir = temp_dir.path().join("logs");

    let mut config = Config::default();
    config.cache_directory = Some(cache_dir.to_string_lossy().to_string());
    config.log_file = Some(log_dir.join("test.log").to_string_lossy().to_string());

    // Ensure directories are created
    config.ensure_directories_exist().unwrap();

    assert!(cache_dir.exists());
    assert!(log_dir.exists());
}

#[tokio::test]
async fn test_config_serialization() {
    let config = Config::default();

    // Test JSON serialization
    let json_str = serde_json::to_string(&config).unwrap();
    let deserialized_config: Config = serde_json::from_str(&json_str).unwrap();

    assert_eq!(config.api_base_url, deserialized_config.api_base_url);
    assert_eq!(config.cache_size_mb, deserialized_config.cache_size_mb);
    assert_eq!(config.default_volume, deserialized_config.default_volume);

    // Test TOML serialization
    let toml_str = toml::to_string(&config).unwrap();
    let toml_deserialized: Config = toml::from_str(&toml_str).unwrap();

    assert_eq!(config.api_base_url, toml_deserialized.api_base_url);
    assert_eq!(config.cache_size_mb, toml_deserialized.cache_size_mb);
    assert_eq!(config.default_volume, toml_deserialized.default_volume);
}

#[tokio::test]
async fn test_config_ui_settings() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("ui_config.toml");

    let config_content = r#"
[ui]
show_cover_art = true
color_scheme = "light"
compact_mode = false
show_equalizer = true
theme = "custom"
font_size = 14
"#;

    fs::write(&config_path, config_content).unwrap();

    let config = Config::load_from_file(&config_path).unwrap();

    if let Some(ui) = config.ui {
        assert!(ui.show_cover_art.unwrap_or(false));
        assert_eq!(ui.color_scheme.unwrap_or_default(), "light");
        assert!(!ui.compact_mode.unwrap_or(true));
        assert!(ui.show_equalizer.unwrap_or(false));
        assert_eq!(ui.theme.unwrap_or_default(), "custom");
        assert_eq!(ui.font_size.unwrap_or(12), 14);
    }
}

#[tokio::test]
async fn test_config_network_settings() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("network_config.toml");

    let config_content = r#"
[network]
timeout_seconds = 60
retry_attempts = 3
user_agent = "DabPlayer/2.0"
proxy_url = "http://proxy.example.com:8080"
enable_ipv6 = false
max_redirects = 10
"#;

    fs::write(&config_path, config_content).unwrap();

    let config = Config::load_from_file(&config_path).unwrap();

    if let Some(network) = config.network {
        assert_eq!(network.timeout_seconds.unwrap_or(30), 60);
        assert_eq!(network.retry_attempts.unwrap_or(3), 3);
        assert_eq!(network.user_agent.unwrap_or_default(), "DabPlayer/2.0");
        assert_eq!(
            network.proxy_url.unwrap_or_default(),
            "http://proxy.example.com:8080"
        );
        assert!(!network.enable_ipv6.unwrap_or(true));
        assert_eq!(network.max_redirects.unwrap_or(5), 10);
    }
}

#[tokio::test]
async fn test_config_audio_settings() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("audio_config.toml");

    let config_content = r#"
[audio]
buffer_size = 8192
sample_rate = 44100
channels = 2
bit_depth = 16
output_device = "default"
enable_equalizer = true
"#;

    fs::write(&config_path, config_content).unwrap();

    let config = Config::load_from_file(&config_path).unwrap();

    if let Some(audio) = config.audio {
        assert_eq!(audio.buffer_size.unwrap_or(4096), 8192);
        assert_eq!(audio.sample_rate.unwrap_or(44100), 44100);
        assert_eq!(audio.channels.unwrap_or(2), 2);
        assert_eq!(audio.bit_depth.unwrap_or(16), 16);
        assert_eq!(audio.output_device.unwrap_or_default(), "default");
        assert!(audio.enable_equalizer.unwrap_or(false));
    }
}

#[tokio::test]
async fn test_config_file_permissions() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("permissions_test.toml");

    let config = Config::default();
    config.save_to_file(&config_path).unwrap();

    // Check that config file has appropriate permissions
    let metadata = fs::metadata(&config_path).unwrap();
    assert!(metadata.is_file());

    // On Unix systems, check that file is readable by owner
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = metadata.permissions();
        assert!(permissions.mode() & 0o400 != 0); // Owner read permission
    }
}
