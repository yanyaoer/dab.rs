use log::{debug, error, info};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::error::{DabError, DabResult};
use crate::player::Track;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub tracks: Vec<DabTrack>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DabTrack {
    pub id: u64,
    pub title: String,
    pub artist: String,
    #[serde(rename = "artistId")]
    pub artist_id: Option<u64>,
    #[serde(rename = "albumTitle")]
    pub album_title: Option<String>,
    #[serde(rename = "albumCover")]
    pub album_cover: Option<String>,
    #[serde(rename = "albumId")]
    pub album_id: Option<String>,
    #[serde(rename = "releaseDate")]
    pub release_date: Option<String>,
    pub genre: Option<String>,
    pub duration: Option<u32>, // Duration in seconds
    #[serde(rename = "audioQuality")]
    pub audio_quality: Option<AudioQuality>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioQuality {
    #[serde(rename = "maximumBitDepth")]
    pub maximum_bit_depth: Option<u32>,
    #[serde(rename = "maximumSamplingRate")]
    pub maximum_sampling_rate: Option<f32>,
    #[serde(rename = "isHiRes")]
    pub is_hi_res: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DabAlbum {
    pub id: String,
    pub title: String,
    pub artist: String,
    #[serde(rename = "releaseDate")]
    pub release_date: Option<String>,
    pub genre: Option<String>,
    pub cover: Option<String>,
    pub tracks: Option<Vec<DabTrack>>,
    #[serde(rename = "trackCount")]
    pub track_count: Option<u32>,
    pub duration: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DabArtist {
    pub id: String,
    pub name: String,
    #[serde(rename = "albumsCount")]
    pub albums_count: Option<u32>,
    pub slug: Option<String>,
    pub image: Option<String>,
    pub biography: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamResponse {
    #[serde(rename = "url")]
    pub stream_url: String,
}

impl From<DabTrack> for Track {
    fn from(dab_track: DabTrack) -> Self {
        Track {
            id: dab_track.id.to_string(),
            title: dab_track.title,
            artist: dab_track.artist,
            album: dab_track
                .album_title
                .unwrap_or_else(|| "Unknown Album".to_string()),
            url: String::new(), // This will be filled by getting stream URL
            duration_ms: dab_track.duration.map(|s| s * 1000).unwrap_or(0),
            local_path: None,
            cover_url: dab_track.album_cover,
        }
    }
}

pub struct DabMusicApi {
    client: Client,
    base_url: String,
}

impl DabMusicApi {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            base_url: "https://dab.yeet.su/api".to_string(),
        }
    }

    pub async fn search(
        &self,
        query: &str,
        search_type: &str,
        limit: u32,
    ) -> DabResult<SearchResult> {
        let url = format!("{}/search", self.base_url);

        info!(
            "Searching DAB API: query='{}', type='{}', limit={}",
            query, search_type, limit
        );

        let response = self
            .client
            .get(&url)
            .query(&[
                ("q", query),
                ("type", search_type),
                ("limit", &limit.to_string()),
            ])
            .send()
            .await?;

        if response.status().is_success() {
            match response.text().await {
                Ok(response_text) => {
                    debug!("Raw API response: {}", response_text);
                    
                    match serde_json::from_str::<SearchResult>(&response_text) {
                        Ok(search_result) => {
                            info!(
                                "DAB API search successful: found {} tracks",
                                search_result.tracks.len()
                            );
                            Ok(search_result)
                        }
                        Err(e) => {
                            error!("Failed to parse DAB API response: {}", e);
                            error!("Response text: {}", response_text);
                            Err(DabError::Network(format!(
                                "Failed to parse DAB API response: {}",
                                e
                            )))
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to read response text: {}", e);
                    Err(DabError::Network(format!(
                        "Failed to read response text: {}",
                        e
                    )))
                }
            }
        } else {
            error!("DAB API returned error: {}", response.status());
            Err(DabError::Network(format!(
                "DAB API returned error: {}",
                response.status()
            )))
        }
    }

    pub async fn get_stream_url(&self, track_id: &str, quality: Option<&str>) -> DabResult<String> {
        let url = format!("{}/stream", self.base_url);
        let quality = quality.unwrap_or("27");

        debug!(
            "Getting stream URL for track: {} with quality: {}",
            track_id, quality
        );

        let response = self
            .client
            .get(&url)
            .query(&[("trackId", track_id), ("quality", quality)])
            .send()
            .await?;

        if response.status().is_success() {
            let response_text = response.text().await.unwrap_or_default();
            debug!("Raw stream API response: {}", response_text);
            
            match serde_json::from_str::<StreamResponse>(&response_text) {
                Ok(stream_response) => {
                    info!(
                        "Got stream URL for track {}: {}",
                        track_id, stream_response.stream_url
                    );
                    Ok(stream_response.stream_url)
                }
                Err(e) => {
                    error!("Failed to parse stream response: {}", e);
                    error!("Stream response text: {}", response_text);
                    Err(DabError::Network(format!(
                        "Failed to parse stream response: {}",
                        e
                    )))
                }
            }
        } else {
            error!("Failed to get stream URL: {}", response.status());
            Err(DabError::Network(format!(
                "Failed to get stream URL: {}",
                response.status()
            )))
        }
    }

    pub async fn get_album_info(&self, album_id: &str) -> DabResult<DabAlbum> {
        let url = format!("{}/album", self.base_url);

        debug!("Getting album info for: {}", album_id);

        let response = self
            .client
            .get(&url)
            .query(&[("albumId", album_id)])
            .send()
            .await?;

        if response.status().is_success() {
            #[derive(Deserialize)]
            struct AlbumResponse {
                album: DabAlbum,
            }

            match response.json::<AlbumResponse>().await {
                Ok(album_response) => {
                    info!("Got album info for: {}", album_id);
                    Ok(album_response.album)
                }
                Err(e) => {
                    error!("Failed to parse album response: {}", e);
                    Err(DabError::Network(format!(
                        "Failed to parse album response: {}",
                        e
                    )))
                }
            }
        } else {
            error!("Failed to get album info: {}", response.status());
            Err(DabError::Network(format!(
                "Failed to get album info: {}",
                response.status()
            )))
        }
    }

    pub async fn get_lyrics(&self, artist: &str, title: &str) -> DabResult<String> {
        let url = format!("{}/lyrics", self.base_url);

        debug!("Getting lyrics for: {} - {}", artist, title);

        let response = self
            .client
            .get(&url)
            .query(&[("artist", artist), ("title", title)])
            .send()
            .await?;

        if response.status().is_success() {
            #[derive(Deserialize)]
            struct LyricsResponse {
                lyrics: String,
                unsynced: Option<bool>,
            }

            match response.json::<LyricsResponse>().await {
                Ok(lyrics_response) => {
                    info!("Got lyrics for: {} - {}", artist, title);
                    Ok(lyrics_response.lyrics)
                }
                Err(e) => {
                    error!("Failed to parse lyrics response: {}", e);
                    Err(DabError::Network(format!(
                        "Failed to parse lyrics response: {}",
                        e
                    )))
                }
            }
        } else if response.status() == 404 {
            Ok("Lyrics not found".to_string())
        } else {
            error!("Failed to get lyrics: {}", response.status());
            Err(DabError::Network(format!(
                "Failed to get lyrics: {}",
                response.status()
            )))
        }
    }
}

// Enhanced search API with convenience methods and cache integration
impl DabMusicApi {
    pub async fn search_tracks(&self, query: &str, limit: u32) -> DabResult<Vec<Track>> {
        self.search_tracks_with_cache(query, limit, None).await
    }

    pub async fn search_tracks_with_cache(
        &self,
        query: &str,
        limit: u32,
        cache: Option<&crate::cache::Cache>,
    ) -> DabResult<Vec<Track>> {
        let search_result = self.search(query, "track", limit).await?;
        let mut tracks = Vec::new();

        for dab_track in search_result.tracks {
            let mut track: Track = dab_track.clone().into();

            // Check cache first if provided
            if let Some(cache) = cache {
                if let Ok(Some(cached_url)) = cache.get_cached_url(&dab_track.id.to_string()).await {
                    if !cached_url.is_empty() {
                        debug!(
                            "Using cached URL for track {}: {}",
                            dab_track.id, cached_url
                        );
                        track.url = cached_url;
                        tracks.push(track);
                        continue;
                    }
                }
            }

            // Try to get the actual stream URL from API
            match self.get_stream_url(&dab_track.id.to_string(), None).await {
                Ok(stream_url) => {
                    track.url = stream_url;
                }
                Err(e) => {
                    error!("Failed to get stream URL for {}: {}", dab_track.id, e);
                    return Err(e);
                }
            }

            tracks.push(track);
        }

        Ok(tracks)
    }

    pub async fn search_albums(&self, query: &str, limit: u32) -> DabResult<Vec<DabAlbum>> {
        let search_result = self.search(query, "album", limit).await?;
        // For now, convert tracks to albums (simplified)
        let albums: Vec<DabAlbum> = search_result
            .tracks
            .into_iter()
            .map(|track| DabAlbum {
                id: track
                    .album_id
                    .unwrap_or_else(|| format!("album_{}", track.id)),
                title: track
                    .album_title
                    .unwrap_or_else(|| "Unknown Album".to_string()),
                artist: track.artist,
                release_date: track.release_date,
                genre: track.genre,
                cover: track.album_cover,
                tracks: None,
                track_count: Some(1),
                duration: track.duration,
            })
            .collect();

        Ok(albums)
    }

    pub async fn search_artists(&self, query: &str, limit: u32) -> DabResult<Vec<DabArtist>> {
        let search_result = self.search(query, "artist", limit).await?;
        // For now, convert tracks to artists (simplified)
        let artists: Vec<DabArtist> = search_result
            .tracks
            .into_iter()
            .map(|track| DabArtist {
                id: track
                    .artist_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| format!("artist_{}", track.id)),
                name: track.artist,
                albums_count: Some(1),
                slug: None,
                image: None,
                biography: None,
            })
            .collect();

        Ok(artists)
    }
}

// For backward compatibility with existing code
pub type MusicSearchApi = DabMusicApi;
