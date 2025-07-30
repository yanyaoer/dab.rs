use log::{debug, info};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::cache::Cache;
use crate::error::{DabError, DabResult};
use crate::player::Track;

#[derive(Debug, Clone)]
pub struct Library {
    cache: Cache,
    metadata: LibraryMetadata,
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

impl Library {
    pub async fn new(cache: Cache) -> DabResult<Self> {
        let metadata = LibraryMetadata {
            artists: HashMap::new(),
            albums: HashMap::new(),
            tracks: HashMap::new(),
        };

        let mut library = Self { cache, metadata };

        // Scan for local music files
        library.scan_local_files().await?;

        info!(
            "Library initialized with {} tracks",
            library.metadata.tracks.len()
        );
        Ok(library)
    }

    pub async fn scan_local_files(&mut self) -> DabResult<()> {
        // TODO: Implement scanning of common music directories
        // For now, just scan cached files
        self.scan_cached_files().await
    }

    async fn scan_cached_files(&mut self) -> DabResult<()> {
        // This would scan cached files and extract metadata using ID3 tags
        info!("Scanning cached files for metadata...");

        // TODO: Implement ID3 tag extraction
        // For now, just log that we're scanning
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
                        url: format!("file://{}", meta.file_path),
                        local_path: Some(meta.file_path.clone()),
                        cover_url: None,
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
                url: format!("file://{}", meta.file_path),
                local_path: Some(meta.file_path.clone()),
                cover_url: None,
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
}
