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
struct Id3Metadata {
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
    id3_metadata: Option<Id3Metadata>,
    unique_id: String,
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
                // Update last accessed time
                entry.last_accessed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                // Save metadata with updated access time
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

        if total_size <= self.max_size_bytes {
            return Ok(());
        }

        info!(
            "Cache size ({} MB) exceeds limit ({} MB), cleaning up old files",
            total_size / (1024 * 1024),
            self.max_size_bytes / (1024 * 1024)
        );

        // Sort entries by last accessed time (oldest first)
        let mut entries: Vec<_> = self
            .metadata
            .entries
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
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

        info!(
            "Cache cleanup completed, new size: {} MB",
            current_size / (1024 * 1024)
        );
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

    async fn save_metadata(&self) -> DabResult<()> {
        let metadata_path = self.cache_dir.join("metadata.json");
        let content = serde_json::to_string_pretty(&self.metadata)?;
        fs::write(&metadata_path, content).await?;
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
        use sha2::{Digest, Sha256};

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
        use sha2::{Digest, Sha256};

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
