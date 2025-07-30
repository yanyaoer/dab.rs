use log::{debug, error, info};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::error::{DabError, DabResult};
use crate::player::Track;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    #[serde(flatten)]
    pub results: SearchResults,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SearchResults {
    Tracks { tracks: Vec<DabTrack> },
    Albums { albums: Vec<DabAlbum> },
    Artists { artists: Vec<DabArtist> },
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
            url: String::new(), // This will be filled when needed for playback
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
                            let result_count = match &search_result.results {
                                SearchResults::Tracks { tracks } => tracks.len(),
                                SearchResults::Albums { albums } => albums.len(),
                                SearchResults::Artists { artists } => artists.len(),
                            };
                            info!(
                                "DAB API search successful: found {} results",
                                result_count
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

    pub async fn get_track_stream_url(
        &self,
        track: &Track,
        cache: Option<&crate::cache::Cache>,
    ) -> DabResult<String> {
        // First check cache if provided
        if let Some(cache) = cache {
            if let Ok(Some(cached_url)) = cache.get_cached_url(&track.id).await {
                if !cached_url.is_empty() {
                    debug!("Using cached URL for track {}: {}", track.id, cached_url);
                    return Ok(cached_url);
                }
            }
        }

        // Get stream URL from API
        let stream_url = self.get_stream_url(&track.id, None).await?;

        // Note: We don't cache the URL directly, we'll cache it when we actually download the track

        Ok(stream_url)
    }

    pub async fn search_tracks_with_cache(
        &self,
        query: &str,
        limit: u32,
        cache: Option<&crate::cache::Cache>,
    ) -> DabResult<Vec<Track>> {
        let search_result = self.search(query, "track", limit).await?;
        let mut tracks = Vec::new();

        match search_result.results {
            SearchResults::Tracks { tracks: dab_tracks } => {
                for dab_track in dab_tracks {
                    let mut track: Track = dab_track.clone().into();

                    // Only check cache, don't fetch stream URL during search
                    if let Some(cache) = cache {
                        if let Ok(Some(cached_url)) = cache.get_cached_url(&dab_track.id.to_string()).await
                        {
                            if !cached_url.is_empty() {
                                debug!(
                                    "Using cached URL for track {}: {}",
                                    dab_track.id, cached_url
                                );
                                track.url = cached_url;
                            }
                        }
                    }

                    tracks.push(track);
                }
            }
            _ => {
                // For compatibility, convert other types to tracks if possible
                debug!("Search result is not tracks, might be albums or artists");
            }
        }

        Ok(tracks)
    }

    pub async fn search_albums(&self, query: &str, limit: u32) -> DabResult<Vec<DabAlbum>> {
        let search_result = self.search(query, "album", limit).await?;
        let mut albums = Vec::new();

        match search_result.results {
            SearchResults::Albums { albums: dab_albums } => {
                albums = dab_albums;
            }
            SearchResults::Tracks { tracks } => {
                // Convert tracks to albums (fallback for backward compatibility)
                for track in tracks {
                    albums.push(DabAlbum {
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
                    });
                }
            }
            _ => {
                debug!("Search result is not albums or tracks");
            }
        }

        Ok(albums)
    }

    pub async fn search_artists(&self, query: &str, limit: u32) -> DabResult<Vec<DabArtist>> {
        let search_result = self.search(query, "artist", limit).await?;
        let mut artists = Vec::new();

        match search_result.results {
            SearchResults::Artists { artists: dab_artists } => {
                artists = dab_artists;
            }
            SearchResults::Tracks { tracks } => {
                // Extract unique artists from tracks and count their albums
                let mut artist_album_count = std::collections::HashMap::new();
                
                for track in tracks {
                    let album_title = track.album_title.as_ref().map(|s| s.as_str()).unwrap_or("Unknown Album");
                    
                    // Count unique albums for each artist
                    let albums_for_artist = artist_album_count
                        .entry(track.artist.clone())
                        .or_insert_with(|| std::collections::HashSet::new());
                    albums_for_artist.insert(album_title.to_string());
                }
                
                // Create artists with proper album counts
                for (artist_name, albums) in artist_album_count {
                    artists.push(DabArtist {
                        id: format!("artist_{}", artist_name.replace(' ', "_")),
                        name: artist_name,
                        albums_count: Some(albums.len() as u32),
                        slug: None,
                        image: None,
                        biography: None,
                    });
                }
            }
            _ => {
                debug!("Search result is not artists or tracks");
            }
        }

        Ok(artists)
    }
}

// For backward compatibility with existing code
pub type MusicSearchApi = DabMusicApi;
