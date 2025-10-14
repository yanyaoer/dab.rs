use log::{debug, info};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::DabResult;
use crate::id_utils::{
    deserialize_id_as_string, deserialize_option_id_as_string, deserialize_similar_artist_ids,
    serialize_id_as_string, serialize_option_id_as_string, serialize_similar_artist_ids,
};
use crate::music_provider::MusicProviderClient;
use crate::player::Track;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    // Handle different response formats from the API
    pub tracks: Option<Vec<DabTrack>>,
    pub albums: Option<Vec<DabAlbum>>,
    pub artists: Option<Vec<DabArtist>>,
    // Fallback for the documented API format
    pub results: Option<Vec<SearchResultItem>>,
}

impl SearchResult {
    /// Get all results in a unified format
    pub fn get_results(&self) -> Vec<SearchResultItem> {
        let mut results = Vec::new();

        // If we have the documented "results" field, use that
        if let Some(ref api_results) = self.results {
            return api_results.clone();
        }

        // Otherwise, collect from the actual API response fields
        if let Some(ref tracks) = self.tracks {
            for track in tracks {
                results.push(SearchResultItem::Track(track.clone()));
            }
        }
        if let Some(ref albums) = self.albums {
            for album in albums {
                results.push(SearchResultItem::Album(album.clone()));
            }
        }
        if let Some(ref artists) = self.artists {
            for artist in artists {
                results.push(SearchResultItem::Artist(artist.clone()));
            }
        }

        results
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SearchResultItem {
    Track(DabTrack),
    Album(DabAlbum),
    Artist(DabArtist),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DabTrack {
    #[serde(
        deserialize_with = "deserialize_id_as_string",
        serialize_with = "serialize_id_as_string"
    )]
    pub id: String,
    pub title: String,
    pub artist: String,
    #[serde(
        rename = "artistId",
        default,
        deserialize_with = "deserialize_option_id_as_string",
        serialize_with = "serialize_option_id_as_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub artist_id: Option<String>,
    #[serde(rename = "albumTitle", default)]
    pub album_title: Option<String>,
    #[serde(rename = "albumCover", default)]
    pub album_cover: Option<String>,
    #[serde(
        rename = "albumId",
        default,
        deserialize_with = "deserialize_option_id_as_string",
        serialize_with = "serialize_option_id_as_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub album_id: Option<String>,
    #[serde(rename = "releaseDate", default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub duration: Option<u32>, // Duration in seconds
    #[serde(rename = "audioQuality", default)]
    pub audio_quality: Option<AudioQuality>,
    // Additional fields present in API responses
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(
        rename = "labelId",
        default,
        deserialize_with = "deserialize_option_id_as_string",
        serialize_with = "serialize_option_id_as_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub label_id: Option<String>,
    #[serde(default)]
    pub upc: Option<String>,
    #[serde(rename = "mediaCount", default)]
    pub media_count: Option<u32>,
    #[serde(rename = "parental_warning", default)]
    pub parental_warning: Option<bool>,
    #[serde(default)]
    pub streamable: Option<bool>,
    #[serde(default)]
    pub purchasable: Option<bool>,
    #[serde(default)]
    pub previewable: Option<bool>,
    #[serde(rename = "genreId", default)]
    pub genre_id: Option<u32>,
    #[serde(rename = "genreSlug", default)]
    pub genre_slug: Option<String>,
    #[serde(rename = "genreColor", default)]
    pub genre_color: Option<String>,
    #[serde(rename = "releaseDateStream", default)]
    pub release_date_stream: Option<String>,
    #[serde(rename = "releaseDateDownload", default)]
    pub release_date_download: Option<String>,
    #[serde(rename = "maximumChannelCount", default)]
    pub maximum_channel_count: Option<u32>,
    #[serde(default)]
    pub images: Option<serde_json::Value>, // Using generic JSON value for flexible image data
    #[serde(default)]
    pub isrc: Option<String>,
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
    #[serde(
        deserialize_with = "deserialize_id_as_string",
        serialize_with = "serialize_id_as_string"
    )]
    pub id: String,
    pub title: String,
    pub artist: String,
    #[serde(
        rename = "artistId",
        default,
        deserialize_with = "deserialize_option_id_as_string",
        serialize_with = "serialize_option_id_as_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub artist_id: Option<String>,
    #[serde(rename = "releaseDate", alias = "release_date")]
    pub release_date: Option<String>,
    pub genre: Option<String>,
    pub cover: Option<String>,
    pub tracks: Option<Vec<DabTrack>>,
    #[serde(rename = "trackCount", alias = "track_count")]
    pub track_count: Option<u32>,
    pub duration: Option<u32>,
    // Additional fields that might be in the API
    #[serde(default)]
    pub label: Option<AlbumLabel>,
    #[serde(default)]
    pub upc: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub streamable: Option<bool>,
    #[serde(default)]
    pub downloadable: Option<bool>,
    #[serde(rename = "mediaCount", alias = "media_count", default)]
    pub media_count: Option<u32>,
    #[serde(
        rename = "maximumChannelCount",
        alias = "maximum_channel_count",
        default
    )]
    pub maximum_channel_count: Option<u32>,
    #[serde(rename = "parental_warning", default)]
    pub parental_warning: Option<bool>,
    #[serde(default)]
    pub popularity: Option<u32>,
    #[serde(rename = "audioQuality", alias = "audio_quality", default)]
    pub audio_quality: Option<AudioQuality>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DabArtist {
    #[serde(
        deserialize_with = "deserialize_id_as_string",
        serialize_with = "serialize_id_as_string"
    )]
    pub id: String,
    pub name: String,
    #[serde(rename = "albumsCount", alias = "albums_count")]
    pub albums_count: Option<u32>,
    #[serde(
        rename = "albumsAsPrimaryArtistCount",
        alias = "albums_as_primary_artist_count",
        default
    )]
    pub albums_as_primary_artist_count: Option<u32>,
    #[serde(
        rename = "albumsAsPrimaryComposerCount",
        alias = "albums_as_primary_composer_count",
        default
    )]
    pub albums_as_primary_composer_count: Option<u32>,
    pub slug: Option<String>,
    pub image: Option<ArtistImage>,
    pub biography: Option<ArtistBiography>,
    #[serde(
        rename = "similarArtistIds",
        alias = "similar_artist_ids",
        default,
        deserialize_with = "deserialize_similar_artist_ids",
        serialize_with = "serialize_similar_artist_ids",
        skip_serializing_if = "Option::is_none"
    )]
    pub similar_artist_ids: Option<Vec<String>>,
    #[serde(default)]
    pub information: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistImage {
    pub small: Option<String>,
    pub medium: Option<String>,
    pub large: Option<String>,
    pub extralarge: Option<String>,
    pub mega: Option<String>,
}

impl std::fmt::Display for ArtistImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref large) = self.large {
            write!(f, "{}", large)
        } else if let Some(ref medium) = self.medium {
            write!(f, "{}", medium)
        } else if let Some(ref small) = self.small {
            write!(f, "{}", small)
        } else {
            write!(f, "No image available")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistBiography {
    pub summary: Option<String>,
    pub content: Option<String>,
}

impl std::fmt::Display for ArtistBiography {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref summary) = self.summary {
            write!(f, "{}", summary)
        } else if let Some(ref content) = self.content {
            // Show first 200 characters of content if no summary
            let truncated = if content.len() > 200 {
                format!("{}...", &content[..200])
            } else {
                content.clone()
            };
            write!(f, "{}", truncated)
        } else {
            write!(f, "No biography available")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumLabel {
    pub name: String,
    #[serde(
        deserialize_with = "deserialize_id_as_string",
        serialize_with = "serialize_id_as_string"
    )]
    pub id: String,
    #[serde(
        deserialize_with = "deserialize_option_id_as_string",
        serialize_with = "serialize_option_id_as_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub albums_count: Option<String>,
    #[serde(
        deserialize_with = "deserialize_option_id_as_string",
        serialize_with = "serialize_option_id_as_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub supplier_id: Option<String>,
    pub slug: Option<String>,
}

impl From<DabTrack> for Track {
    fn from(dab_track: DabTrack) -> Self {
        Track {
            id: dab_track.id.clone(),
            title: dab_track.title,
            artist: dab_track.artist,
            album: dab_track
                .album_title
                .unwrap_or_else(|| "Unknown Album".to_string()),
            duration_ms: dab_track.duration.map(|s| s * 1000).unwrap_or(0),
            local_path: None,
            cover_url: dab_track.album_cover,
            track_id: Some(dab_track.id),
            artist_id: dab_track.artist_id,
            album_id: dab_track.album_id,
        }
    }
}

#[derive(Clone)]
pub struct DabMusicApi {
    provider: MusicProviderClient,
}

impl DabMusicApi {
    pub fn new() -> Self {
        Self {
            provider: MusicProviderClient::new(),
        }
    }

    pub fn new_with_config(config: &Config) -> Self {
        Self {
            provider: MusicProviderClient::new_with_config(config.clone()),
        }
    }

    pub async fn search(
        &self,
        query: &str,
        search_type: &str,
        limit: u32,
    ) -> DabResult<SearchResult> {
        info!(
            "Searching API: query='{}', type='{}', limit={}",
            query, search_type, limit
        );

        self.provider.search(query, search_type, limit).await
    }

    pub async fn get_stream_url(
        &self,
        track_id: &str,
        _quality: Option<&str>,
    ) -> DabResult<String> {
        debug!("Getting stream URL for track: {}", track_id);
        self.provider.get_stream_url(track_id).await
    }

    pub async fn get_album_info(&self, album_id: &str) -> DabResult<DabAlbum> {
        debug!("Getting album info for: {}", album_id);
        self.provider.get_album_info(album_id).await
    }

    pub async fn get_lyrics(&self, _artist: &str, _title: &str) -> DabResult<String> {
        // TODO: Add lyrics support to backend
        Ok("Lyrics not available".to_string())
    }

    pub async fn get_artist_discography(
        &self,
        artist_id: &str,
    ) -> DabResult<(DabArtist, Vec<DabAlbum>)> {
        debug!("Getting discography for artist: {}", artist_id);
        self.provider.get_artist_discography(artist_id).await
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
        _cache: Option<&crate::cache::Cache>,
    ) -> DabResult<Vec<Track>> {
        let search_result = self.search(query, "track", limit).await?;
        let mut tracks = Vec::new();

        for item in search_result.get_results() {
            match item {
                SearchResultItem::Track(dab_track) => {
                    let track: Track = dab_track.clone().into();

                    // Only check cache, don't fetch stream URL during search
                    // Cache checking now handled by PlayerEngine during playback
                    tracks.push(track);
                }
                _ => {
                    debug!("Skipping non-track item in track search");
                }
            }
        }

        Ok(tracks)
    }

    pub async fn search_albums(&self, query: &str, limit: u32) -> DabResult<Vec<DabAlbum>> {
        let search_result = self.search(query, "album", limit).await?;
        let mut albums = Vec::new();

        for item in search_result.get_results() {
            match item {
                SearchResultItem::Album(dab_album) => {
                    albums.push(dab_album);
                }
                SearchResultItem::Track(track) => {
                    // Convert tracks to albums (fallback for backward compatibility)
                    albums.push(DabAlbum {
                        id: track.album_id.unwrap_or_else(|| track.id.clone()),
                        title: track
                            .album_title
                            .unwrap_or_else(|| "Unknown Album".to_string()),
                        artist: track.artist.clone(),
                        artist_id: track.artist_id.clone(),
                        release_date: track.release_date,
                        genre: track.genre,
                        cover: track.album_cover,
                        tracks: None,
                        track_count: Some(1),
                        duration: track.duration,
                        label: None,
                        upc: None,
                        url: None,
                        streamable: None,
                        downloadable: None,
                        media_count: None,
                        maximum_channel_count: None,
                        parental_warning: None,
                        popularity: None,
                        audio_quality: None,
                    });
                }
                _ => {
                    debug!("Skipping non-album/track item in album search");
                }
            }
        }

        Ok(albums)
    }

    pub async fn search_artists(&self, query: &str, limit: u32) -> DabResult<Vec<DabArtist>> {
        let search_result = self.search(query, "artist", limit).await?;
        let mut artists = Vec::new();

        for item in search_result.get_results() {
            match item {
                SearchResultItem::Artist(dab_artist) => {
                    artists.push(dab_artist);
                }
                SearchResultItem::Track(track) => {
                    // Extract unique artists from tracks and count their albums
                    let mut artist_album_count = std::collections::HashMap::new();

                    let album_title = track
                        .album_title
                        .as_ref()
                        .map(|s| s.as_str())
                        .unwrap_or("Unknown Album");

                    // Count unique albums for each artist
                    let albums_for_artist = artist_album_count
                        .entry(track.artist.clone())
                        .or_insert_with(|| std::collections::HashSet::new());
                    albums_for_artist.insert(album_title.to_string());

                    // Create artist with proper album counts
                    for (artist_name, albums) in artist_album_count {
                        artists.push(DabArtist {
                            id: artist_name.clone(), // Use artist name as ID for generated artists
                            name: artist_name,
                            albums_count: Some(albums.len() as u32),
                            albums_as_primary_artist_count: None,
                            albums_as_primary_composer_count: None,
                            slug: None,
                            image: None,
                            biography: None,
                            similar_artist_ids: None,
                            information: None,
                        });
                    }
                }
                _ => {
                    debug!("Skipping non-artist/track item in artist search");
                }
            }
        }

        Ok(artists)
    }
}

// For backward compatibility with existing code
pub type MusicSearchApi = DabMusicApi;
