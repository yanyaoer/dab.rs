use std::path::{Path, PathBuf};
use std::collections::HashMap;
use tokio::fs;
use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use log::{info, debug, warn};

use crate::error::{DabResult, DabError};
use crate::config::Config;

#[derive(Debug, Clone)]
pub struct Cache {
    cache_dir: PathBuf,
    max_size_bytes: u64,
    metadata: CacheMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheMetadata {
    version: u32,
    entries: HashMap<String, CacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    track_id: String,
    file_path: String,
    size_bytes: u64,
    created_at: u64,
    last_accessed: u64,
    url: String,
}

impl Cache {
    pub async fn new() -> DabResult<Self> {
        let config = Config::load();
        let cache_dir = PathBuf::from(&config.cache_dir);
        let max_size_bytes = config.max_cache_size_mb * 1024 * 1024;
        
        // Create cache directory if it doesn't exist
        if !cache_dir.exists() {
            fs::create_dir_all(&cache_dir).await?;
            info!("Created cache directory: {}", cache_dir.display());
        }
        
        let metadata_path = cache_dir.join("metadata.json");
        let metadata = if metadata_path.exists() {
            let content = fs::read_to_string(&metadata_path).await?;
            serde_json::from_str(&content).unwrap_or_else(|e| {
                warn!("Failed to parse cache metadata, starting fresh: {}", e);
                CacheMetadata::new()
            })
        } else {
            CacheMetadata::new()
        };
        
        let cache = Self {
            cache_dir,
            max_size_bytes,
            metadata,
        };
        
        // Clean up any orphaned files
        cache.cleanup().await?;
        
        info!("Cache initialized at: {}", cache.cache_dir.display());
        Ok(cache)
    }
    
    pub async fn has_track(&self, track_id: &str) -> DabResult<bool> {
        Ok(self.metadata.entries.contains_key(track_id))
    }
    
    pub async fn get_track_path(&self, track_id: &str) -> DabResult<Option<PathBuf>> {
        if let Some(entry) = self.metadata.entries.get(track_id) {
            let path = PathBuf::from(&entry.file_path);
            if path.exists() {
                // Update last accessed time
                // Note: In a real implementation, we'd update this in the metadata file
                Ok(Some(path))
            } else {
                // File was deleted, remove from metadata
                warn!("Cached file missing, removing from metadata: {}", path.display());
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
    
    pub async fn store_track(&mut self, track_id: &str, source_path: &Path) -> DabResult<PathBuf> {
        let file_extension = source_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("audio");
            
        let cache_filename = format!("{}.{}", track_id, file_extension);
        let cache_path = self.cache_dir.join(&cache_filename);
        
        // Copy file to cache
        fs::copy(source_path, &cache_path).await?;
        
        let metadata = fs::metadata(&cache_path).await?;
        let size_bytes = metadata.len();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        // Add to metadata
        let entry = CacheEntry {
            track_id: track_id.to_string(),
            file_path: cache_path.to_string_lossy().to_string(),
            size_bytes,
            created_at: now,
            last_accessed: now,
            url: String::new(), // TODO: Store original URL
        };
        
        self.metadata.entries.insert(track_id.to_string(), entry);
        
        // Check if we need to clean up old files
        self.enforce_size_limit().await?;
        
        // Save metadata
        self.save_metadata().await?;
        
        info!("Stored track in cache: {} ({} bytes)", cache_path.display(), size_bytes);
        Ok(cache_path)
    }
    
    pub async fn remove_track(&mut self, track_id: &str) -> DabResult<()> {
        if let Some(entry) = self.metadata.entries.remove(track_id) {
            let path = PathBuf::from(&entry.file_path);
            if path.exists() {
                fs::remove_file(&path).await?;
                debug!("Removed cached file: {}", path.display());
            }
            self.save_metadata().await?;
        }
        Ok(())
    }
    
    pub async fn clear_all(&mut self) -> DabResult<()> {
        // Remove all cached files
        for entry in self.metadata.entries.values() {
            let path = PathBuf::from(&entry.file_path);
            if path.exists() {
                if let Err(e) = fs::remove_file(&path).await {
                    warn!("Failed to remove cached file {}: {}", path.display(), e);
                }
            }
        }
        
        self.metadata.entries.clear();
        self.save_metadata().await?;
        
        info!("Cleared all cached files");
        Ok(())
    }
    
    pub fn get_cache_size(&self) -> u64 {
        self.metadata.entries.values()
            .map(|entry| entry.size_bytes)
            .sum()
    }
    
    pub fn get_track_count(&self) -> usize {
        self.metadata.entries.len()
    }
    
    async fn enforce_size_limit(&mut self) -> DabResult<()> {
        let total_size = self.get_cache_size();
        
        if total_size <= self.max_size_bytes {
            return Ok(());
        }
        
        info!(
            "Cache size ({} MB) exceeds limit ({} MB), cleaning up old files",
            total_size / (1024 * 1024),
            self.max_size_bytes / (1024 * 1024)
        );
        
        // Sort entries by last accessed time (oldest first)
        let mut entries: Vec<_> = self.metadata.entries.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        entries.sort_by_key(|(_, entry)| entry.last_accessed);
        
        let mut current_size = total_size;
        let target_size = (self.max_size_bytes as f64 * 0.8) as u64; // Clean up to 80% of limit
        
        for (track_id, entry) in entries {
            if current_size <= target_size {
                break;
            }
            
            current_size -= entry.size_bytes;
            let path = PathBuf::from(&entry.file_path);
            
            if path.exists() {
                fs::remove_file(&path).await?;
                debug!("Removed old cached file: {}", path.display());
            }
            
            self.metadata.entries.remove(&track_id);
        }
        
        info!("Cache cleanup completed, new size: {} MB", current_size / (1024 * 1024));
        Ok(())
    }
    
    async fn cleanup(&self) -> DabResult<()> {
        // Remove any files in cache directory that aren't in metadata
        let mut entries = fs::read_dir(&self.cache_dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            
            if path.file_name().and_then(|n| n.to_str()) == Some("metadata.json") {
                continue;
            }
            
            if path.is_file() {
                let path_str = path.to_string_lossy();
                let is_tracked = self.metadata.entries.values()
                    .any(|entry| entry.file_path == path_str);
                    
                if !is_tracked {
                    if let Err(e) = fs::remove_file(&path).await {
                        warn!("Failed to remove orphaned file {}: {}", path.display(), e);
                    } else {
                        debug!("Removed orphaned cached file: {}", path.display());
                    }
                }
            }
        }
        
        Ok(())
    }
    
    async fn save_metadata(&self) -> DabResult<()> {
        let metadata_path = self.cache_dir.join("metadata.json");
        let content = serde_json::to_string_pretty(&self.metadata)?;
        fs::write(&metadata_path, content).await?;
        Ok(())
    }
}

impl CacheMetadata {
    fn new() -> Self {
        Self {
            version: 1,
            entries: HashMap::new(),
        }
    }
}