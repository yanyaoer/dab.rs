use id3::TagLike;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::cache::Cache;
use crate::error::DabResult;
use crate::player::Track;
use crate::search::DabAlbum;

#[derive(Debug, Clone)]
pub struct Library {
    cache: Cache,
    metadata: LibraryMetadata,
    favorites: FavoriteAlbums,
    favorites_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LibraryMetadata {
    artists: HashMap<String, Artist>,
    albums: HashMap<String, Album>,
    tracks: HashMap<String, TrackMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artist {
    pub name: String,
    pub albums: Vec<String>,
    pub track_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Album {
    pub title: String,
    pub artist: String,
    pub tracks: Vec<String>,
    pub year: Option<u32>,
    pub cover_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrackMetadata {
    id: String,
    title: String,
    artist: String,
    album: String,
    duration_ms: Option<u32>,
    track_number: Option<u32>,
    year: Option<u32>,
    genre: Option<String>,
    file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FavoriteAlbums {
    albums: HashMap<String, FavoriteAlbum>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FavoriteAlbum {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub artist_id: Option<String>,
    pub cover: Option<String>,
    pub release_date: Option<String>,
    pub track_count: Option<u32>,
    pub added_at: chrono::DateTime<chrono::Utc>,
}

impl From<&DabAlbum> for FavoriteAlbum {
    fn from(album: &DabAlbum) -> Self {
        Self {
            id: album.id.clone(),
            title: album.title.clone(),
            artist: album.artist.clone(),
            artist_id: album.artist_id.clone(),
            cover: album.cover.clone(),
            release_date: album.release_date.clone(),
            track_count: album.track_count,
            added_at: chrono::Utc::now(),
        }
    }
}

impl Library {
    pub async fn new(mut cache: Cache) -> DabResult<Self> {
        let metadata = LibraryMetadata {
            artists: HashMap::new(),
            albums: HashMap::new(),
            tracks: HashMap::new(),
        };

        let cache_dir = cache.get_cache_dir();
        let favorites_path = cache_dir.join("favorite_albums.json");
        let favorites = Self::load_favorites(&favorites_path).unwrap_or_default();

        // Auto-pin all cached tracks from existing favorite albums
        let favorite_album_ids: Vec<String> = favorites
            .albums
            .values()
            .map(|album| album.id.clone())
            .collect();

        if !favorite_album_ids.is_empty() {
            let pinned = cache.pin_tracks_by_album_ids(&favorite_album_ids).await?;
            if pinned > 0 {
                info!(
                    "Auto-pinned {} cached tracks from {} existing favorite albums",
                    pinned,
                    favorite_album_ids.len()
                );
            }
        }

        let mut library = Self {
            cache,
            metadata,
            favorites,
            favorites_path,
        };

        // Scan for local music files
        library.scan_local_files().await?;

        info!(
            "Library initialized with {} tracks and {} favorite albums",
            library.metadata.tracks.len(),
            library.favorites.albums.len()
        );
        Ok(library)
    }

    pub async fn scan_local_files(&mut self) -> DabResult<()> {
        // TODO: Implement scanning of common music directories
        // For now, just scan cached files
        self.scan_cached_files().await
    }

    async fn scan_cached_files(&mut self) -> DabResult<()> {
        info!("Scanning cached files for metadata...");
        let cached_files = self.cache.get_all_cached_files().await?;

        for (track_id, file_path) in cached_files {
            if let Ok(tag) = id3::Tag::read_from_path(&file_path) {
                let title = tag.title().unwrap_or("Unknown Title").to_string();
                let artist = tag.artist().unwrap_or("Unknown Artist").to_string();
                let album = tag.album().unwrap_or("Unknown Album").to_string();
                let duration_ms = tag.duration().map(|d| d * 1000);

                let metadata = TrackMetadata {
                    id: track_id.clone(),
                    title,
                    artist: artist.clone(),
                    album: album.clone(),
                    duration_ms,
                    track_number: tag.track(),
                    year: tag.year().map(|y| y as u32),
                    genre: tag.genre().map(|s| s.to_string()),
                    file_path: file_path.to_string_lossy().to_string(),
                };

                self.metadata.tracks.insert(track_id.clone(), metadata);
                self.update_artist_album_metadata(&artist, &album, &track_id);
            }
        }

        debug!("Cached files scanned");
        Ok(())
    }

    pub fn get_artists(&self) -> Vec<&Artist> {
        self.metadata.artists.values().collect()
    }

    pub fn get_albums(&self) -> Vec<&Album> {
        self.metadata.albums.values().collect()
    }

    pub fn get_albums_by_artist(&self, artist_name: &str) -> Vec<&Album> {
        if let Some(artist) = self.metadata.artists.get(artist_name) {
            artist
                .albums
                .iter()
                .filter_map(|album_id| self.metadata.albums.get(album_id))
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn get_tracks_by_album(&self, album_title: &str) -> Vec<Track> {
        if let Some(album) = self
            .metadata
            .albums
            .values()
            .find(|a| a.title == album_title)
        {
            album
                .tracks
                .iter()
                .filter_map(|track_id| {
                    self.metadata.tracks.get(track_id).map(|meta| Track {
                        id: meta.id.clone(),
                        title: meta.title.clone(),
                        artist: meta.artist.clone(),
                        album: meta.album.clone(),
                        duration_ms: meta.duration_ms.unwrap_or(0),
                        local_path: Some(format!("file://{}", meta.file_path)),
                        cover_url: None,
                        track_id: None,
                        artist_id: None,
                        album_id: None,
                    })
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn search_tracks(&self, query: &str) -> Vec<Track> {
        let query_lower = query.to_lowercase();

        self.metadata
            .tracks
            .values()
            .filter(|meta| {
                meta.title.to_lowercase().contains(&query_lower)
                    || meta.artist.to_lowercase().contains(&query_lower)
                    || meta.album.to_lowercase().contains(&query_lower)
            })
            .map(|meta| Track {
                id: meta.id.clone(),
                title: meta.title.clone(),
                artist: meta.artist.clone(),
                album: meta.album.clone(),
                duration_ms: meta.duration_ms.unwrap_or(0),
                local_path: Some(format!("file://{}", meta.file_path)),
                cover_url: None,
                track_id: None,
                artist_id: None,
                album_id: None,
            })
            .collect()
    }

    pub async fn add_track_metadata(&mut self, track: &Track, file_path: &str) -> DabResult<()> {
        // Extract metadata using ID3 tags if available
        let metadata = TrackMetadata {
            id: track.id.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_ms: Some(track.duration_ms),
            track_number: None,
            year: None,
            genre: None,
            file_path: file_path.to_string(),
        };

        self.metadata.tracks.insert(track.id.clone(), metadata);

        // Update artist and album collections
        self.update_artist_album_metadata(&track.artist, &track.album, &track.id);

        Ok(())
    }

    fn update_artist_album_metadata(
        &mut self,
        artist_name: &str,
        album_title: &str,
        track_id: &str,
    ) {
        // Update or create artist
        self.metadata
            .artists
            .entry(artist_name.to_string())
            .and_modify(|artist| {
                artist.track_count += 1;
                if !artist.albums.contains(&album_title.to_string()) {
                    artist.albums.push(album_title.to_string());
                }
            })
            .or_insert_with(|| Artist {
                name: artist_name.to_string(),
                albums: vec![album_title.to_string()],
                track_count: 1,
            });

        // Update or create album
        self.metadata
            .albums
            .entry(album_title.to_string())
            .and_modify(|album| {
                if !album.tracks.contains(&track_id.to_string()) {
                    album.tracks.push(track_id.to_string());
                }
            })
            .or_insert_with(|| Album {
                title: album_title.to_string(),
                artist: artist_name.to_string(),
                tracks: vec![track_id.to_string()],
                year: None,
                cover_path: None,
            });
    }

    // Favorites management methods

    fn load_favorites(path: &PathBuf) -> Option<FavoriteAlbums> {
        match fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(favorites) => Some(favorites),
                Err(e) => {
                    warn!("Failed to parse favorites file: {}", e);
                    None
                }
            },
            Err(_) => None, // File doesn't exist yet
        }
    }

    fn save_favorites(&self) -> DabResult<()> {
        if let Some(parent) = self.favorites_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(&self.favorites)?;
        fs::write(&self.favorites_path, content)?;

        debug!(
            "Saved {} favorite albums to {:?}",
            self.favorites.albums.len(),
            self.favorites_path
        );
        Ok(())
    }

    pub async fn add_favorite_album(&mut self, album: &DabAlbum) -> DabResult<bool> {
        let favorite_album = FavoriteAlbum::from(album);
        let album_key = format!("{}-{}", album.artist, album.title);

        let is_new = !self.favorites.albums.contains_key(&album_key);
        self.favorites
            .albums
            .insert(album_key.clone(), favorite_album);

        self.save_favorites()?;

        // Auto-pin all cached tracks from this favorite album
        let pinned = self
            .cache
            .pin_tracks_by_album_ids(&[album.id.clone()])
            .await?;
        if pinned > 0 {
            info!(
                "Auto-pinned {} cached tracks from favorite album '{}'",
                pinned, album.title
            );
        }

        info!(
            "Added album '{}' by '{}' to favorites",
            album.title, album.artist
        );
        Ok(is_new)
    }

    pub async fn remove_favorite_album(&mut self, artist: &str, title: &str) -> DabResult<bool> {
        let album_key = format!("{}-{}", artist, title);

        // Get album_id before removing
        let album_id = self
            .favorites
            .albums
            .get(&album_key)
            .map(|album| album.id.clone());

        let removed = self.favorites.albums.remove(&album_key).is_some();

        if removed {
            self.save_favorites()?;

            // Auto-unpin all cached tracks from this removed favorite album
            if let Some(id) = album_id {
                let unpinned = self.cache.unpin_tracks_by_album_ids(&[id]).await?;
                if unpinned > 0 {
                    info!(
                        "Auto-unpinned {} cached tracks from removed favorite album '{}'",
                        unpinned, title
                    );
                }
            }

            info!("Removed album '{}' by '{}' from favorites", title, artist);
        }

        Ok(removed)
    }

    pub fn get_favorite_albums(&self) -> Vec<&FavoriteAlbum> {
        let mut albums: Vec<&FavoriteAlbum> = self.favorites.albums.values().collect();
        // Sort by added date, most recent first
        albums.sort_by(|a, b| b.added_at.cmp(&a.added_at));
        albums
    }

    pub fn is_favorite_album(&self, artist: &str, title: &str) -> bool {
        let album_key = format!("{}-{}", artist, title);
        self.favorites.albums.contains_key(&album_key)
    }
}
