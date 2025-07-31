use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;

use crate::config::Config;
use crate::error::{DabError, DabResult};
use id3::TagLike;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Id3Metadata {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    year: Option<u32>,
    genre: Option<String>,
    duration_ms: Option<u32>,
    track_number: Option<u32>,
    total_tracks: Option<u32>,
    album_artist: Option<String>,
    composer: Option<String>,
    comment: Option<String>,
    cover_art_hash: Option<String>,
}

/// Cache status for tracks
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CacheStatus {
    NotCached,
    PartiallyDownloaded { progress: f32 }, // 0.0-1.0
    FullyDownloaded,
    StreamReady, // Has enough buffer to start playback
}

#[derive(Debug, Clone)]
pub struct Cache {
    cache_dir: PathBuf,
    max_size_bytes: u64,
    metadata: CacheMetadata,
    // New cache management settings
    max_age_days: u32,          // Maximum age for low priority tracks
    min_free_space_mb: u64,     // Minimum free space to maintain
    preload_next_tracks: u32,   // Number of next tracks to preload
    stream_url_expire_hours: u32, // Hours after which stream URLs expire
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
    id3_metadata: Option<Id3Metadata>,
    unique_id: String,
    download_status: CacheStatus,
    expected_size: Option<u64>, // For partial downloads
    // New fields for improved cache management
    access_count: u64,
    priority: CachePriority,
    last_played: Option<u64>,
    expires_at: Option<u64>, // For URL-based tracks that may expire
}

/// Priority levels for cache entries
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum CachePriority {
    Low,        // Rarely accessed tracks
    Normal,     // Default priority
    High,       // Frequently played tracks
    Pinned,     // Never delete (user favorites, current queue)
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

        let mut cache = Self {
            cache_dir,
            max_size_bytes,
            metadata,
            max_age_days: config.cache_max_age_days,
            min_free_space_mb: config.cache_min_free_space_mb,
            preload_next_tracks: config.preload_next_tracks,
            stream_url_expire_hours: config.stream_url_expire_hours,
        };

        // Clean up any orphaned files and expired entries
        cache.cleanup().await?;
        cache.cleanup_expired().await?;

        info!("Cache initialized at: {}", cache.cache_dir.display());
        Ok(cache)
    }

    pub async fn has_track(&self, track_id: &str) -> DabResult<bool> {
        if let Some(entry) = self.metadata.entries.get(track_id) {
            let path = PathBuf::from(&entry.file_path);
            Ok(path.exists())
        } else {
            Ok(false)
        }
    }

    pub async fn get_cached_url(&self, track_id: &str) -> DabResult<Option<String>> {
        if let Some(entry) = self.metadata.entries.get(track_id) {
            let path = PathBuf::from(&entry.file_path);
            if path.exists() && !entry.url.is_empty() {
                Ok(Some(entry.url.clone()))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    pub async fn get_track_path(&mut self, track_id: &str) -> DabResult<Option<PathBuf>> {
        if let Some(entry) = self.metadata.entries.get_mut(track_id) {
            let path = PathBuf::from(&entry.file_path);
            if path.exists() {
                // Update access statistics
                entry.last_accessed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                entry.access_count += 1;
                
                // Auto-promote frequently accessed tracks
                if entry.access_count >= 10 && entry.priority == CachePriority::Normal {
                    entry.priority = CachePriority::High;
                    info!("Promoted track {} to high priority (access count: {})", track_id, entry.access_count);
                }
                
                // Save metadata with updated access stats
                let _ = self.save_metadata().await;
                Ok(Some(path))
            } else {
                // File was deleted, remove from metadata
                warn!(
                    "Cached file missing, removing from metadata: {}",
                    path.display()
                );
                self.metadata.entries.remove(track_id);
                let _ = self.save_metadata().await;
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    pub async fn store_track(&mut self, track_id: &str, source_path: &Path) -> DabResult<PathBuf> {
        self.store_track_with_url(track_id, source_path, "").await
    }

    pub async fn store_track_with_url(
        &mut self,
        track_id: &str,
        source_path: &Path,
        original_url: &str,
    ) -> DabResult<PathBuf> {
        let file_extension = source_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("mp3");

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

        // Extract ID3 metadata from the source file
        let id3_metadata = self.extract_id3_metadata(source_path).await.unwrap_or(None);

        // Generate unique ID based on file content hash and metadata
        let unique_id = self
            .generate_unique_id(track_id, &id3_metadata, original_url)
            .await;

        // Add to metadata
        let entry = CacheEntry {
            track_id: track_id.to_string(),
            file_path: cache_path.to_string_lossy().to_string(),
            size_bytes,
            created_at: now,
            last_accessed: now,
            url: original_url.to_string(),
            id3_metadata,
            unique_id,
            download_status: CacheStatus::FullyDownloaded,
            expected_size: Some(size_bytes),
            // Initialize new fields
            access_count: 1, // First access when storing
            priority: CachePriority::Normal,
            last_played: None,
            expires_at: if original_url.starts_with("http") {
                Some(now + (self.stream_url_expire_hours as u64 * 3600))
            } else {
                None // Local files don't expire
            },
        };

        self.metadata.entries.insert(track_id.to_string(), entry);

        // Check if we need to clean up old files
        self.enforce_size_limit().await?;

        // Save metadata
        self.save_metadata().await?;

        info!(
            "Stored track in cache: {} ({} bytes)",
            cache_path.display(),
            size_bytes
        );
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
        self.metadata
            .entries
            .values()
            .map(|entry| entry.size_bytes)
            .sum()
    }

    pub fn get_track_count(&self) -> usize {
        self.metadata.entries.len()
    }

    pub async fn get_all_cached_files(&self) -> DabResult<HashMap<String, PathBuf>> {
        let mut files = HashMap::new();
        for (track_id, entry) in &self.metadata.entries {
            files.insert(track_id.clone(), PathBuf::from(&entry.file_path));
        }
        Ok(files)
    }

    async fn enforce_size_limit(&mut self) -> DabResult<()> {
        let total_size = self.get_cache_size();
        let target_size = self.max_size_bytes - (self.min_free_space_mb * 1024 * 1024);

        if total_size <= target_size {
            return Ok(());
        }

        info!(
            "Cache size ({} MB) exceeds limit, cleaning up with smart strategy",
            total_size / (1024 * 1024)
        );

        // Multi-tier cleanup strategy
        let candidates = self.get_cleanup_candidates().await;
        let mut current_size = total_size;
        let final_target_size = (target_size as f64 * 0.8) as u64; // Clean up to 80% of target

        for (track_id, entry) in candidates {
            if current_size <= final_target_size {
                break;
            }

            // Never delete pinned tracks
            if entry.priority == CachePriority::Pinned {
                continue;
            }

            current_size -= entry.size_bytes;
            let path = PathBuf::from(&entry.file_path);

            if path.exists() {
                fs::remove_file(&path).await?;
                debug!("Removed cached file: {} (priority: {:?}, access_count: {})", 
                       path.display(), entry.priority, entry.access_count);
            }

            self.metadata.entries.remove(&track_id);
        }

        info!(
            "Smart cache cleanup completed, new size: {} MB, freed: {} MB",
            current_size / (1024 * 1024),
            (total_size - current_size) / (1024 * 1024)
        );
        Ok(())
    }

    /// Get cleanup candidates sorted by priority (least important first)
    async fn get_cleanup_candidates(&self) -> Vec<(String, CacheEntry)> {
        let mut candidates: Vec<_> = self
            .metadata
            .entries
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // Sort by cleanup priority (complex scoring system)
        candidates.sort_by(|a, b| {
            let score_a = self.calculate_cleanup_score(&a.1);
            let score_b = self.calculate_cleanup_score(&b.1);
            score_a.partial_cmp(&score_b).unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates
    }

    /// Calculate cleanup score (lower score = higher cleanup priority)
    fn calculate_cleanup_score(&self, entry: &CacheEntry) -> f64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Base score by priority
        let mut score = match entry.priority {
            CachePriority::Pinned => 1000.0,  // Never delete
            CachePriority::High => 100.0,
            CachePriority::Normal => 50.0,
            CachePriority::Low => 10.0,
        };

        // Factor in access frequency (higher = better score)
        score += (entry.access_count as f64).log10() * 10.0;

        // Factor in recency of access (more recent = better score)
        let days_since_access = (now - entry.last_accessed) as f64 / (24.0 * 3600.0);
        score -= days_since_access * 2.0;

        // Factor in last played (more recent = better score)
        if let Some(last_played) = entry.last_played {
            let days_since_played = (now - last_played) as f64 / (24.0 * 3600.0);
            score -= days_since_played;
        }

        // Penalty for very old files
        let days_since_created = (now - entry.created_at) as f64 / (24.0 * 3600.0);
        if days_since_created > self.max_age_days as f64 && entry.priority == CachePriority::Low {
            score -= 50.0; // Heavy penalty for old low-priority files
        }

        score.max(0.0)
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
                let is_tracked = self
                    .metadata
                    .entries
                    .values()
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

    /// Clean up expired cache entries (for stream URLs)
    async fn cleanup_expired(&mut self) -> DabResult<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut expired_tracks = Vec::new();

        for (track_id, entry) in &self.metadata.entries {
            if let Some(expires_at) = entry.expires_at {
                if now > expires_at {
                    expired_tracks.push(track_id.clone());
                }
            }
        }

        if !expired_tracks.is_empty() {
            info!("Cleaning up {} expired cache entries", expired_tracks.len());
            for track_id in expired_tracks {
                if let Some(entry) = self.metadata.entries.remove(&track_id) {
                    let path = PathBuf::from(&entry.file_path);
                    if path.exists() {
                        if let Err(e) = fs::remove_file(&path).await {
                            warn!("Failed to remove expired cached file {}: {}", path.display(), e);
                        } else {
                            debug!("Removed expired cached file: {}", path.display());
                        }
                    }
                }
            }
            self.save_metadata().await?;
        }

        Ok(())
    }

    async fn save_metadata(&self) -> DabResult<()> {
        let metadata_path = self.cache_dir.join("metadata.json");
        let content = serde_json::to_string_pretty(&self.metadata)?;
        fs::write(&metadata_path, content).await?;
        Ok(())
    }

    /// Mark track as played (updates last_played timestamp)
    pub async fn mark_track_played(&mut self, track_id: &str) -> DabResult<()> {
        if let Some(entry) = self.metadata.entries.get_mut(track_id) {
            entry.last_played = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            );
            self.save_metadata().await?;
        }
        Ok(())
    }

    /// Set cache priority for a track
    pub async fn set_track_priority(&mut self, track_id: &str, priority: CachePriority) -> DabResult<()> {
        if let Some(entry) = self.metadata.entries.get_mut(track_id) {
            let old_priority = entry.priority.clone();
            entry.priority = priority;
            info!("Changed track {} priority from {:?} to {:?}", track_id, old_priority, entry.priority);
            self.save_metadata().await?;
        }
        Ok(())
    }

    /// Pin tracks to prevent deletion (e.g., current queue, favorites)
    pub async fn pin_tracks(&mut self, track_ids: &[String]) -> DabResult<()> {
        for track_id in track_ids {
            if let Some(entry) = self.metadata.entries.get_mut(track_id) {
                entry.priority = CachePriority::Pinned;
            }
        }
        self.save_metadata().await?;
        info!("Pinned {} tracks to prevent deletion", track_ids.len());
        Ok(())
    }

    /// Unpin tracks (restore to normal priority)
    pub async fn unpin_tracks(&mut self, track_ids: &[String]) -> DabResult<()> {
        for track_id in track_ids {
            if let Some(entry) = self.metadata.entries.get_mut(track_id) {
                if entry.priority == CachePriority::Pinned {
                    entry.priority = CachePriority::Normal;
                }
            }
        }
        self.save_metadata().await?;
        info!("Unpinned {} tracks", track_ids.len());
        Ok(())
    }

    /// Get cache statistics
    pub fn get_cache_stats(&self) -> CacheStats {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut stats = CacheStats {
            total_tracks: self.metadata.entries.len(),
            total_size_bytes: 0,
            pinned_tracks: 0,
            high_priority_tracks: 0,
            normal_priority_tracks: 0,
            low_priority_tracks: 0,
            expired_tracks: 0,
            average_access_count: 0.0,
        };

        let mut total_access_count = 0u64;

        for entry in self.metadata.entries.values() {
            stats.total_size_bytes += entry.size_bytes;
            total_access_count += entry.access_count;

            match entry.priority {
                CachePriority::Pinned => stats.pinned_tracks += 1,
                CachePriority::High => stats.high_priority_tracks += 1,
                CachePriority::Normal => stats.normal_priority_tracks += 1,
                CachePriority::Low => stats.low_priority_tracks += 1,
            }

            if let Some(expires_at) = entry.expires_at {
                if now > expires_at {
                    stats.expired_tracks += 1;
                }
            }
        }

        if stats.total_tracks > 0 {
            stats.average_access_count = total_access_count as f64 / stats.total_tracks as f64;
        }

        stats
    }

    /// Periodic maintenance (should be called regularly)
    pub async fn perform_maintenance(&mut self) -> DabResult<()> {
        info!("Performing cache maintenance");
        
        // Clean up expired entries
        self.cleanup_expired().await?;
        
        // Perform smart cleanup if needed
        self.enforce_size_limit().await?;
        
        // Auto-demote rarely accessed tracks
        self.auto_adjust_priorities().await?;
        
        info!("Cache maintenance completed");
        Ok(())
    }

    /// Auto-adjust track priorities based on access patterns
    async fn auto_adjust_priorities(&mut self) -> DabResult<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut changes = 0;

        for entry in self.metadata.entries.values_mut() {
            let days_since_access = (now - entry.last_accessed) as f64 / (24.0 * 3600.0);
            
            // Demote high priority tracks that haven't been accessed in a while
            if entry.priority == CachePriority::High && days_since_access > 7.0 && entry.access_count < 5 {
                entry.priority = CachePriority::Normal;
                changes += 1;
            }
            
            // Demote normal priority tracks that are very old and rarely accessed
            if entry.priority == CachePriority::Normal && days_since_access > 14.0 && entry.access_count < 3 {
                entry.priority = CachePriority::Low;
                changes += 1;
            }
        }

        if changes > 0 {
            info!("Auto-adjusted priorities for {} tracks", changes);
            self.save_metadata().await?;
        }

        Ok(())
    }

    /// Extract ID3 metadata from audio file
    async fn extract_id3_metadata(&self, file_path: &Path) -> DabResult<Option<Id3Metadata>> {
        if !file_path.exists() {
            return Ok(None);
        }

        match id3::Tag::read_from_path(file_path) {
            Ok(tag) => {
                let metadata = Id3Metadata {
                    title: tag.title().map(|s| s.to_string()),
                    artist: tag.artist().map(|s| s.to_string()),
                    album: tag.album().map(|s| s.to_string()),
                    year: tag.year().map(|y| y as u32),
                    genre: tag.genre().map(|s| s.to_string()),
                    duration_ms: None, // ID3 doesn't store duration, will be set during playback
                    track_number: tag.track().map(|t| t as u32),
                    total_tracks: tag.total_tracks().map(|t| t as u32),
                    album_artist: tag.album_artist().map(|s| s.to_string()),
                    composer: None, // Composer field not available in id3::Tag
                    comment: tag.comments().next().map(|c| c.text.to_string()),
                    cover_art_hash: None, // TODO: Extract and hash cover art
                };
                Ok(Some(metadata))
            }
            Err(e) => {
                debug!(
                    "Failed to read ID3 tags from {}: {}",
                    file_path.display(),
                    e
                );
                Ok(None)
            }
        }
    }

    /// Generate unique ID for track based on content and metadata
    async fn generate_unique_id(
        &self,
        track_id: &str,
        id3_metadata: &Option<Id3Metadata>,
        url: &str,
    ) -> String {
        let mut hasher = Sha256::new();

        // Include track ID
        hasher.update(track_id.as_bytes());

        // Include URL
        hasher.update(url.as_bytes());

        // Include ID3 metadata if available
        if let Some(metadata) = id3_metadata {
            if let Some(ref title) = metadata.title {
                hasher.update(title.as_bytes());
            }
            if let Some(ref artist) = metadata.artist {
                hasher.update(artist.as_bytes());
            }
            if let Some(ref album) = metadata.album {
                hasher.update(album.as_bytes());
            }
        }

        let result = hasher.finalize();
        format!("{:x}", result)
    }

    /// Get track ID3 metadata
    pub async fn get_track_metadata(&self, track_id: &str) -> DabResult<Option<Id3Metadata>> {
        if let Some(entry) = self.metadata.entries.get(track_id) {
            Ok(entry.id3_metadata.clone())
        } else {
            Ok(None)
        }
    }

    /// Get track by unique ID
    pub async fn get_track_by_unique_id(&self, unique_id: &str) -> DabResult<Option<String>> {
        for (track_id, entry) in &self.metadata.entries {
            if entry.unique_id == unique_id {
                return Ok(Some(track_id.clone()));
            }
        }
        Ok(None)
    }

    /// Check if track is cached by unique ID
    pub async fn has_track_by_unique_id(&self, unique_id: &str) -> DabResult<bool> {
        self.get_track_by_unique_id(unique_id)
            .await
            .map(|opt| opt.is_some())
    }

    /// Search cached tracks by metadata
    pub async fn search_cached_tracks(
        &self,
        query: &SearchQuery,
    ) -> DabResult<Vec<CachedTrackInfo>> {
        let mut results = Vec::new();

        for (track_id, entry) in &self.metadata.entries {
            let path = PathBuf::from(&entry.file_path);
            if !path.exists() {
                continue;
            }

            let matches = match query {
                SearchQuery::Title(title_query) => entry
                    .id3_metadata
                    .as_ref()
                    .and_then(|m| m.title.as_ref())
                    .map(|t| t.to_lowercase().contains(&title_query.to_lowercase()))
                    .unwrap_or(false),
                SearchQuery::Artist(artist_query) => entry
                    .id3_metadata
                    .as_ref()
                    .and_then(|m| m.artist.as_ref())
                    .map(|a| a.to_lowercase().contains(&artist_query.to_lowercase()))
                    .unwrap_or(false),
                SearchQuery::Album(album_query) => entry
                    .id3_metadata
                    .as_ref()
                    .and_then(|m| m.album.as_ref())
                    .map(|a| a.to_lowercase().contains(&album_query.to_lowercase()))
                    .unwrap_or(false),
                SearchQuery::General(query) => {
                    let query_lower = query.to_lowercase();
                    let title_matches = entry
                        .id3_metadata
                        .as_ref()
                        .and_then(|m| m.title.as_ref())
                        .map(|t| t.to_lowercase().contains(&query_lower))
                        .unwrap_or(false);
                    let artist_matches = entry
                        .id3_metadata
                        .as_ref()
                        .and_then(|m| m.artist.as_ref())
                        .map(|a| a.to_lowercase().contains(&query_lower))
                        .unwrap_or(false);
                    let album_matches = entry
                        .id3_metadata
                        .as_ref()
                        .and_then(|m| m.album.as_ref())
                        .map(|a| a.to_lowercase().contains(&query_lower))
                        .unwrap_or(false);
                    title_matches || artist_matches || album_matches
                }
            };

            if matches {
                results.push(CachedTrackInfo {
                    track_id: track_id.clone(),
                    file_path: path.clone(),
                    metadata: entry.id3_metadata.clone(),
                    unique_id: entry.unique_id.clone(),
                    url: entry.url.clone(),
                    size_bytes: entry.size_bytes,
                    last_accessed: entry.last_accessed,
                });
            }
        }

        Ok(results)
    }

    /// Get all tracks for an album
    pub async fn get_album_tracks(&self, album_name: &str) -> DabResult<Vec<CachedTrackInfo>> {
        let mut tracks = Vec::new();

        for (track_id, entry) in &self.metadata.entries {
            let path = PathBuf::from(&entry.file_path);
            if !path.exists() {
                continue;
            }

            if let Some(ref metadata) = entry.id3_metadata {
                if let Some(ref album) = metadata.album {
                    if album.to_lowercase() == album_name.to_lowercase() {
                        tracks.push(CachedTrackInfo {
                            track_id: track_id.clone(),
                            file_path: path.clone(),
                            metadata: Some(metadata.clone()),
                            unique_id: entry.unique_id.clone(),
                            url: entry.url.clone(),
                            size_bytes: entry.size_bytes,
                            last_accessed: entry.last_accessed,
                        });
                    }
                }
            }
        }

        // Sort by track number if available
        tracks.sort_by(|a, b| {
            let a_track_num = a
                .metadata
                .as_ref()
                .and_then(|m| m.track_number)
                .unwrap_or(0);
            let b_track_num = b
                .metadata
                .as_ref()
                .and_then(|m| m.track_number)
                .unwrap_or(0);
            a_track_num.cmp(&b_track_num)
        });

        Ok(tracks)
    }

    /// Get all albums from cache
    pub async fn get_cached_albums(&self) -> DabResult<Vec<AlbumInfo>> {
        let mut albums: HashMap<String, AlbumInfo> = HashMap::new();

        for entry in self.metadata.entries.values() {
            let path = PathBuf::from(&entry.file_path);
            if !path.exists() {
                continue;
            }

            if let Some(ref metadata) = entry.id3_metadata {
                if let Some(ref album_name) = metadata.album {
                    let album_info =
                        albums
                            .entry(album_name.clone())
                            .or_insert_with(|| AlbumInfo {
                                name: album_name.clone(),
                                artist: metadata
                                    .album_artist
                                    .clone()
                                    .or_else(|| metadata.artist.clone()),
                                year: metadata.year,
                                genre: metadata.genre.clone(),
                                track_count: 0,
                                total_size_bytes: 0,
                                cover_art_hash: metadata.cover_art_hash.clone(),
                            });

                    album_info.track_count += 1;
                    album_info.total_size_bytes += entry.size_bytes;
                }
            }
        }

        Ok(albums.into_values().collect())
    }

    /// Update track metadata with new information
    pub async fn update_track_metadata(
        &mut self,
        track_id: &str,
        new_metadata: Id3Metadata,
    ) -> DabResult<()> {
        if let Some(entry) = self.metadata.entries.get_mut(track_id) {
            entry.id3_metadata = Some(new_metadata);
            // Regenerate unique ID with updated metadata
            let unique_id =
                Self::generate_unique_id_static(track_id, &entry.id3_metadata, &entry.url).await;
            entry.unique_id = unique_id;
            self.save_metadata().await?;
            Ok(())
        } else {
            Err(DabError::Cache(format!(
                "Track {} not found in cache",
                track_id
            )))
        }
    }

    /// Static version of generate_unique_id for use in methods that need to avoid borrowing conflicts
    async fn generate_unique_id_static(
        track_id: &str,
        id3_metadata: &Option<Id3Metadata>,
        url: &str,
    ) -> String {
        let mut hasher = Sha256::new();

        // Include track ID
        hasher.update(track_id.as_bytes());

        // Include URL
        hasher.update(url.as_bytes());

        // Include ID3 metadata if available
        if let Some(metadata) = id3_metadata {
            if let Some(ref title) = metadata.title {
                hasher.update(title.as_bytes());
            }
            if let Some(ref artist) = metadata.artist {
                hasher.update(artist.as_bytes());
            }
            if let Some(ref album) = metadata.album {
                hasher.update(album.as_bytes());
            }
        }

        let result = hasher.finalize();
        format!("{:x}", result)
    }

    /// Get cache status for a track
    pub async fn get_cache_status(&self, track_id: &str) -> CacheStatus {
        if let Some(entry) = self.metadata.entries.get(track_id) {
            let path = PathBuf::from(&entry.file_path);
            if path.exists() {
                entry.download_status.clone()
            } else {
                CacheStatus::NotCached
            }
        } else {
            CacheStatus::NotCached
        }
    }

    /// Update download status for a track
    pub async fn update_download_status(&mut self, track_id: &str, status: CacheStatus) -> DabResult<()> {
        if let Some(entry) = self.metadata.entries.get_mut(track_id) {
            entry.download_status = status;
            self.save_metadata().await?;
            Ok(())
        } else {
            Err(DabError::Cache(format!(
                "Track {} not found in cache",
                track_id
            )))
        }
    }

    /// Update download progress for a track
    pub async fn update_download_progress(&mut self, track_id: &str, progress: f32) -> DabResult<()> {
        if let Some(entry) = self.metadata.entries.get_mut(track_id) {
            entry.download_status = if progress >= 1.0 {
                CacheStatus::FullyDownloaded
            } else if progress >= 0.1 { // 10% threshold for stream ready
                CacheStatus::StreamReady
            } else {
                CacheStatus::PartiallyDownloaded { progress }
            };
            self.save_metadata().await?;
            Ok(())
        } else {
            // Create a new entry for partial download
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            let entry = CacheEntry {
                track_id: track_id.to_string(),
                file_path: String::new(), // Will be set when download completes
                size_bytes: 0,
                created_at: now,
                last_accessed: now,
                url: String::new(),
                id3_metadata: None,
                unique_id: String::new(),
                download_status: CacheStatus::PartiallyDownloaded { progress },
                expected_size: None,
                // Initialize new fields
                access_count: 0,
                priority: CachePriority::Normal,
                last_played: None,
                expires_at: None,
            };

            self.metadata.entries.insert(track_id.to_string(), entry);
            self.save_metadata().await?;
            Ok(())
        }
    }

    /// Check if track is ready for streaming (has enough buffer)
    pub async fn is_stream_ready(&self, track_id: &str, min_buffer_mb: u32) -> bool {
        if let Some(entry) = self.metadata.entries.get(track_id) {
            match &entry.download_status {
                CacheStatus::FullyDownloaded => true,
                CacheStatus::StreamReady => true,
                CacheStatus::PartiallyDownloaded { progress } => {
                    if let Some(expected_size) = entry.expected_size {
                        let downloaded_bytes = (expected_size as f32 * progress) as u64;
                        let min_buffer_bytes = (min_buffer_mb as u64) * 1024 * 1024;
                        downloaded_bytes >= min_buffer_bytes
                    } else {
                        *progress >= 0.1 // Default 10% threshold
                    }
                }
                CacheStatus::NotCached => false,
            }
        } else {
            false
        }
    }

    /// Get download progress for a track (0.0 to 1.0)
    pub async fn get_download_progress(&self, track_id: &str) -> Option<f32> {
        if let Some(entry) = self.metadata.entries.get(track_id) {
            match &entry.download_status {
                CacheStatus::FullyDownloaded => Some(1.0),
                CacheStatus::StreamReady => Some(0.5), // Estimate
                CacheStatus::PartiallyDownloaded { progress } => Some(*progress),
                CacheStatus::NotCached => Some(0.0),
            }
        } else {
            None
        }
    }

    /// Set expected size for a track (used when starting downloads)
    pub async fn set_expected_size(&mut self, track_id: &str, expected_size: u64) -> DabResult<()> {
        if let Some(entry) = self.metadata.entries.get_mut(track_id) {
            entry.expected_size = Some(expected_size);
            self.save_metadata().await?;
        }
        Ok(())
    }

    /// Start partial download entry
    pub async fn start_partial_download(&mut self, track_id: &str, url: &str, expected_size: Option<u64>) -> DabResult<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let entry = CacheEntry {
            track_id: track_id.to_string(),
            file_path: String::new(), // Will be set when download completes
            size_bytes: 0,
            created_at: now,
            last_accessed: now,
            url: url.to_string(),
            id3_metadata: None,
            unique_id: String::new(),
            download_status: CacheStatus::PartiallyDownloaded { progress: 0.0 },
            expected_size,
            // Initialize new fields
            access_count: 0,
            priority: CachePriority::Normal,
            last_played: None,
            expires_at: if url.starts_with("http") {
                Some(now + (self.stream_url_expire_hours as u64 * 3600))
            } else {
                None
            },
        };

        self.metadata.entries.insert(track_id.to_string(), entry);
        self.save_metadata().await?;
        info!("Started partial download for track: {}", track_id);
        Ok(())
    }

    /// Complete partial download (convert to full cache entry)
    pub async fn complete_partial_download(
        &mut self,
        track_id: &str,
        file_path: &Path,
        actual_size: u64,
    ) -> DabResult<PathBuf> {
        // Get entry URL before mutation
        let entry_url = if let Some(entry) = self.metadata.entries.get(track_id) {
            entry.url.clone()
        } else {
            return Err(DabError::Cache(format!(
                "Partial download entry not found for track: {}",
                track_id
            )));
        };

        // Move file to proper cache location
        let file_extension = file_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("mp3");

        let cache_filename = format!("{}.{}", track_id, file_extension);
        let cache_path = self.cache_dir.join(&cache_filename);

        // Copy file to cache
        fs::copy(file_path, &cache_path).await?;

        // Extract ID3 metadata
        let id3_metadata = self.extract_id3_metadata(&cache_path).await.unwrap_or(None);
        
        // Generate unique ID
        let unique_id = Self::generate_unique_id_static(track_id, &id3_metadata, &entry_url).await;

        // Now update the entry
        if let Some(entry) = self.metadata.entries.get_mut(track_id) {
            entry.file_path = cache_path.to_string_lossy().to_string();
            entry.size_bytes = actual_size;
            entry.download_status = CacheStatus::FullyDownloaded;
            entry.last_accessed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            entry.id3_metadata = id3_metadata;
            entry.unique_id = unique_id;

            self.save_metadata().await?;
            info!("Completed partial download for track: {} -> {}", track_id, cache_path.display());
            Ok(cache_path)
        } else {
            Err(DabError::Cache(format!(
                "Partial download entry not found for track: {}",
                track_id
            )))
        }
    }
    
    pub fn get_cache_dir(&self) -> &PathBuf {
        &self.cache_dir
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

#[derive(Debug, Clone)]
pub enum SearchQuery {
    Title(String),
    Artist(String),
    Album(String),
    General(String),
}

#[derive(Debug, Clone)]
pub struct CachedTrackInfo {
    pub track_id: String,
    pub file_path: PathBuf,
    pub metadata: Option<Id3Metadata>,
    pub unique_id: String,
    pub url: String,
    pub size_bytes: u64,
    pub last_accessed: u64,
}

#[derive(Debug, Clone)]
pub struct AlbumInfo {
    pub name: String,
    pub artist: Option<String>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    pub track_count: usize,
    pub total_size_bytes: u64,
    pub cover_art_hash: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub total_tracks: usize,
    pub total_size_bytes: u64,
    pub pinned_tracks: usize,
    pub high_priority_tracks: usize,
    pub normal_priority_tracks: usize,
    pub low_priority_tracks: usize,
    pub expired_tracks: usize,
    pub average_access_count: f64,
}
