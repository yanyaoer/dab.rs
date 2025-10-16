use log::{debug, error, info};
use reqwest::{Client, Url};
use serde::Deserialize;
use std::time::Duration;

use crate::cache::Cache;
use crate::config::{ApiTarget, AudioQuality, Config};
use crate::error::{DabError, DabResult};
use crate::library::Library;
use crate::search::{DabAlbum, DabArtist, DabTrack, SearchResult};

/// Search request parameters
#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub query: String,
    pub search_type: SearchType,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone)]
pub enum SearchType {
    Track,
    Album,
    Artist,
}

impl SearchType {
    fn as_str(&self) -> &str {
        match self {
            SearchType::Track => "track",
            SearchType::Album => "album",
            SearchType::Artist => "artist",
        }
    }

    fn to_squid_param(&self) -> &str {
        match self {
            SearchType::Track => "s",
            SearchType::Album => "al",
            SearchType::Artist => "a",
        }
    }
}

impl From<&str> for SearchType {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "album" => SearchType::Album,
            "artist" => SearchType::Artist,
            _ => SearchType::Track,
        }
    }
}

/// Music provider client for Squid API with automatic failover
#[derive(Clone)]
pub struct MusicProviderClient {
    client: Client,
    config: Config,
    cache: Option<Cache>,
    library: Option<Library>,
}

impl MusicProviderClient {
    pub fn new() -> Self {
        let config = Config::load();
        let client = Client::builder()
            .timeout(Duration::from_secs(config.api.timeout_seconds))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            config,
            cache: None,
            library: None,
        }
    }

    pub fn new_with_config(config: Config) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.api.timeout_seconds))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            config,
            cache: None,
            library: None,
        }
    }

    pub async fn new_with_cache_and_library(config: Config) -> DabResult<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.api.timeout_seconds))
            .build()
            .unwrap_or_else(|_| Client::new());

        let cache = Cache::new().await.ok();
        let library = if let Some(ref cache) = cache {
            Library::new(cache.clone()).await.ok()
        } else {
            None
        };

        Ok(Self {
            client,
            config,
            cache,
            library,
        })
    }

    /// Set cache reference (useful when cache is created after provider)
    pub fn set_cache(&mut self, cache: Cache) {
        self.cache = Some(cache);
    }

    /// Set library reference (useful when library is created after provider)
    pub fn set_library(&mut self, library: Library) {
        self.library = Some(library);
    }

    /// Perform a GET request with fallback and return raw response text
    async fn fetch_text_with_fallback<F>(
        &self,
        operation: &str,
        request_builder: F,
    ) -> DabResult<String>
    where
        F: Fn(&ApiTarget) -> String,
    {
        let targets = self.config.api.targets_by_weight();
        let mut last_error = None;

        for (attempt, target) in targets.iter().enumerate() {
            let url = request_builder(target);

            debug!(
                "Attempting {} with target '{}' (attempt {}/{}) : {}",
                operation,
                target.name,
                attempt + 1,
                targets.len(),
                url
            );

            let mut request = self.client.get(&url);

            if target.requires_proxy && self.config.api.use_proxy {
                if let Some(ref proxy_url) = self.config.api.proxy_url {
                    let encoded_url = urlencoding::encode(&url);
                    let proxied_url = format!("{}?url={}", proxy_url, encoded_url);
                    request = self.client.get(&proxied_url);
                }
            }

            match request.send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        match response.text().await {
                            Ok(body) => {
                                if !body.trim().is_empty() {
                                    info!("{} successful with target '{}'", operation, target.name);
                                    return Ok(body);
                                } else {
                                    error!(
                                        "{} returned empty body from target '{}' - URL: {}",
                                        operation, target.name, url
                                    );
                                    last_error =
                                        Some(DabError::Network("Empty response body".to_string()));
                                }
                            }
                            Err(e) => {
                                error!(
                                    "{} read error with target '{}': {} - URL: {}",
                                    operation, target.name, e, url
                                );
                                last_error = Some(DabError::Network(format!("Read error: {}", e)));
                            }
                        }
                    } else {
                        error!(
                            "{} HTTP error with target '{}': {} - URL: {}",
                            operation,
                            target.name,
                            response.status(),
                            url
                        );
                        last_error = Some(DabError::Network(format!(
                            "HTTP error: {}",
                            response.status()
                        )));
                    }
                }
                Err(e) => {
                    error!(
                        "{} network error with target '{}': {} - URL: {}",
                        operation, target.name, e, url
                    );
                    last_error = Some(DabError::Network(format!("Network error: {}", e)));
                }
            }

            if attempt < targets.len() - 1 {
                debug!("Retrying with next target...");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }

        Err(last_error
            .unwrap_or_else(|| DabError::Network(format!("{} failed with all targets", operation))))
    }

    async fn fetch_with_fallback<F, T>(&self, operation: &str, request_builder: F) -> DabResult<T>
    where
        F: Fn(&ApiTarget) -> String,
        T: for<'de> Deserialize<'de>,
    {
        let targets = self.config.api.targets_by_weight();
        let mut last_error = None;

        for (attempt, target) in targets.iter().enumerate() {
            let url = request_builder(target);

            debug!(
                "Attempting {} with target '{}' (attempt {}/{}): {}",
                operation,
                target.name,
                attempt + 1,
                targets.len(),
                url
            );

            let mut request = self.client.get(&url);

            // Add proxy if needed
            if target.requires_proxy && self.config.api.use_proxy {
                if let Some(ref proxy_url) = self.config.api.proxy_url {
                    let encoded_url = urlencoding::encode(&url);
                    let proxied_url = format!("{}?url={}", proxy_url, encoded_url);
                    request = self.client.get(&proxied_url);
                }
            }

            match request.send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        match response.json::<T>().await {
                            Ok(data) => {
                                info!("{} successful with target '{}'", operation, target.name);
                                return Ok(data);
                            }
                            Err(e) => {
                                error!(
                                    "{} parse error with target '{}': {} - URL: {}",
                                    operation, target.name, e, url
                                );
                                last_error = Some(DabError::Network(format!("Parse error: {}", e)));
                            }
                        }
                    } else {
                        error!(
                            "{} HTTP error with target '{}': {} - URL: {}",
                            operation,
                            target.name,
                            response.status(),
                            url
                        );
                        last_error = Some(DabError::Network(format!(
                            "HTTP error: {}",
                            response.status()
                        )));
                    }
                }
                Err(e) => {
                    error!(
                        "{} network error with target '{}': {} - URL: {}",
                        operation, target.name, e, url
                    );
                    last_error = Some(DabError::Network(format!("Network error: {}", e)));
                }
            }

            // Try next target
            if attempt < targets.len() - 1 {
                debug!("Retrying with next target...");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }

        Err(last_error
            .unwrap_or_else(|| DabError::Network(format!("{} failed with all targets", operation))))
    }

    fn looks_like_stream_url(url: &str) -> bool {
        url.starts_with("http://") || url.starts_with("https://")
    }

    fn extract_stream_url(value: &serde_json::Value) -> Option<&str> {
        const PRIMARY_KEYS: [&str; 3] = ["OriginalTrackUrl", "originalTrackUrl", "streamUrl"];

        match value {
            serde_json::Value::String(s) => {
                if Self::looks_like_stream_url(s) {
                    Some(s)
                } else {
                    None
                }
            }
            serde_json::Value::Object(map) => {
                // Check primary keys first
                for key in PRIMARY_KEYS.iter() {
                    if let Some(serde_json::Value::String(s)) = map.get(*key) {
                        if Self::looks_like_stream_url(s) {
                            return Some(s);
                        }
                    }
                }

                // Check generic "url" field
                if let Some(serde_json::Value::String(url)) = map.get("url") {
                    if Self::looks_like_stream_url(url) {
                        return Some(url);
                    }
                }

                // Recursively search other fields (skip manifest)
                for (key, value) in map {
                    if key.eq_ignore_ascii_case("manifest") {
                        continue;
                    }
                    if let Some(url) = Self::extract_stream_url(value) {
                        return Some(url);
                    }
                }
                None
            }
            serde_json::Value::Array(entries) => {
                // Search array entries (reverse order for priority)
                for entry in entries.iter().rev() {
                    if let Some(url) = Self::extract_stream_url(entry) {
                        return Some(url);
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn map_quality_to_param(&self) -> &'static str {
        match self.config.audio_quality {
            AudioQuality::Low => "LOW",
            AudioQuality::Medium => "HIGH",
            AudioQuality::High => "LOSSLESS",
        }
    }

    fn validate_stream_url(raw: &str) -> DabResult<String> {
        let trimmed = raw.trim();
        let parsed = Url::parse(trimmed)
            .map_err(|e| DabError::Network(format!("Invalid stream URL '{}': {}", trimmed, e)))?;
        Ok(parsed.into())
    }

    /// Search for music (tracks, albums, artists)
    pub async fn search(
        &self,
        query: &str,
        search_type: &str,
        limit: u32,
    ) -> DabResult<SearchResult> {
        let request = SearchRequest {
            query: query.to_string(),
            search_type: SearchType::from(search_type),
            limit: Some(limit),
        };
        self.search_with_request(&request).await
    }

    /// Search for music using structured request
    pub async fn search_with_request(&self, request: &SearchRequest) -> DabResult<SearchResult> {
        // Squid API uses different query parameters:
        // Track search: /search/?s=query
        // Album search: /search/?al=query
        // Artist search: /search/?a=query

        // Common structures used across all search types
        #[derive(Deserialize)]
        struct SquidSimpleArtist {
            id: i64,
            name: String,
        }

        #[derive(Deserialize)]
        struct SquidSimpleAlbum {
            id: i64,
            title: String,
            cover: Option<String>,
        }

        // Track structure used in all response types
        #[derive(Deserialize)]
        struct SquidTrack {
            id: i64,
            title: String,
            duration: Option<u32>,
            #[serde(rename = "trackNumber")]
            track_number: Option<u32>,
            artist: Option<SquidSimpleArtist>,
            artists: Option<Vec<SquidSimpleArtist>>,
            album: Option<SquidSimpleAlbum>,
        }

        #[derive(Deserialize)]
        struct SquidAlbum {
            id: i64,
            title: String,
            duration: Option<u32>,
            #[serde(rename = "numberOfTracks")]
            number_of_tracks: Option<u32>,
            #[serde(rename = "releaseDate")]
            release_date: Option<String>,
            cover: Option<String>,
            artist: Option<SquidSimpleArtist>,
            artists: Option<Vec<SquidSimpleArtist>>,
        }

        #[derive(Deserialize)]
        struct SquidArtist {
            id: i64,
            name: String,
            picture: Option<String>,
            #[serde(flatten)]
            _extra: std::collections::HashMap<String, serde_json::Value>,
        }

        #[derive(Deserialize)]
        struct SquidItemList<T> {
            items: Vec<T>,
            #[serde(rename = "totalNumberOfItems")]
            total_number_of_items: Option<u32>,
        }

        // Unified response structure
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum SquidSearchResponse {
            // Track search returns items directly
            DirectTrackList(SquidItemList<SquidTrack>),
            // Artist search returns array with nested response
            ArrayResponse(Vec<SquidNestedResponse>),
            // Album search returns nested structure
            NestedResponse(SquidNestedResponse),
        }

        #[derive(Deserialize)]
        struct SquidNestedResponse {
            tracks: Option<SquidItemList<SquidTrack>>,
            albums: Option<SquidItemList<SquidAlbum>>,
            artists: Option<SquidItemList<SquidArtist>>,
        }

        // Helper function to convert Squid track to DAB track
        fn convert_track(t: SquidTrack) -> DabTrack {
            // Extract artist info - try singular artist first, then fallback to artists array
            let (artist_name, artist_id) = if let Some(artist) = &t.artist {
                (artist.name.clone(), Some(artist.id.to_string()))
            } else if let Some(artists) = &t.artists {
                if let Some(first_artist) = artists.first() {
                    (first_artist.name.clone(), Some(first_artist.id.to_string()))
                } else {
                    (String::new(), None)
                }
            } else {
                (String::new(), None)
            };

            DabTrack {
                id: t.id.to_string(),
                title: t.title,
                artist: artist_name,
                artist_id,
                album_title: t.album.as_ref().map(|a| a.title.clone()),
                album_cover: t
                    .album
                    .as_ref()
                    .and_then(|a| a.cover.as_ref().map(|c| format_cover_url(c))),
                album_id: t.album.map(|a| a.id.to_string()),
                release_date: None,
                genre: None,
                duration: t.duration,
                audio_quality: None,
                version: None,
                label: None,
                label_id: None,
                upc: None,
                media_count: None,
                parental_warning: None,
                streamable: Some(true),
                purchasable: None,
                previewable: None,
                genre_id: None,
                genre_slug: None,
                genre_color: None,
                release_date_stream: None,
                release_date_download: None,
                maximum_channel_count: None,
                images: None,
                isrc: None,
            }
        }

        // Helper function to format cover URLs
        fn format_cover_url(cover: &str) -> String {
            if cover.starts_with("http") {
                cover.to_string()
            } else {
                format!(
                    "https://resources.tidal.com/images/{}/1280x1280.jpg",
                    cover.replace('-', "/")
                )
            }
        }

        // Helper function to format artist picture URLs
        fn format_artist_picture(picture: &str) -> crate::search::ArtistImage {
            let url = if picture.starts_with("http") {
                picture.to_string()
            } else {
                format!(
                    "https://resources.tidal.com/images/{}/750x750.jpg",
                    picture.replace('-', "/")
                )
            };
            crate::search::ArtistImage {
                small: Some(url.clone()),
                medium: Some(url.clone()),
                large: Some(url.clone()),
                extralarge: Some(url.clone()),
                mega: Some(url),
            }
        }

        let response: SquidSearchResponse = self
            .fetch_with_fallback("Squid Search", |target| {
                // Use the correct Squid API parameter based on search type
                let param = request.search_type.to_squid_param();
                format!(
                    "{}/search/?{}={}",
                    target.base_url,
                    param,
                    urlencoding::encode(&request.query)
                )
            })
            .await?;

        // Transform Squid response to unified format
        let mut search_result = SearchResult {
            tracks: None,
            albums: None,
            artists: None,
            results: None,
        };

        match response {
            SquidSearchResponse::DirectTrackList(track_list) => {
                // Handle direct track list response (from ?s= search)
                let tracks: Vec<DabTrack> =
                    track_list.items.into_iter().map(convert_track).collect();
                search_result.tracks = Some(tracks);
            }
            SquidSearchResponse::ArrayResponse(array) => {
                // Handle array response (from ?a= artist search)
                if let Some(nested) = array.first() {
                    if let Some(squid_artists) = &nested.artists {
                        let artists: Vec<DabArtist> = squid_artists
                            .items
                            .iter()
                            .map(|a| DabArtist {
                                id: a.id.to_string(),
                                name: a.name.clone(),
                                albums_count: None,
                                albums_as_primary_artist_count: None,
                                albums_as_primary_composer_count: None,
                                slug: None,
                                image: a.picture.as_ref().map(|pic| format_artist_picture(pic)),
                                biography: None,
                                similar_artist_ids: None,
                                information: None,
                            })
                            .collect();
                        search_result.artists = Some(artists);
                    }
                }
            }
            SquidSearchResponse::NestedResponse(nested) => {
                // Handle nested response (from ?al= album search)
                if let Some(squid_tracks) = nested.tracks {
                    let tracks: Vec<DabTrack> =
                        squid_tracks.items.into_iter().map(convert_track).collect();
                    search_result.tracks = Some(tracks);
                }

                if let Some(squid_albums) = nested.albums {
                    let albums: Vec<DabAlbum> = squid_albums
                        .items
                        .into_iter()
                        .map(|a| {
                            // Extract artist info - try singular artist first, then fallback to artists array
                            let (artist_name, artist_id) = if let Some(artist) = &a.artist {
                                (artist.name.clone(), Some(artist.id.to_string()))
                            } else if let Some(artists) = &a.artists {
                                if let Some(first_artist) = artists.first() {
                                    (first_artist.name.clone(), Some(first_artist.id.to_string()))
                                } else {
                                    (String::new(), None)
                                }
                            } else {
                                (String::new(), None)
                            };

                            DabAlbum {
                                id: a.id.to_string(),
                                title: a.title,
                                artist: artist_name,
                                artist_id,
                                release_date: a.release_date,
                                genre: None,
                                cover: a.cover.map(|c| format_cover_url(&c)),
                                tracks: None,
                                track_count: a.number_of_tracks,
                                duration: a.duration,
                                label: None,
                                upc: None,
                                url: None,
                                streamable: Some(true),
                                downloadable: None,
                                media_count: None,
                                maximum_channel_count: None,
                                parental_warning: None,
                                popularity: None,
                                audio_quality: None,
                            }
                        })
                        .collect();
                    search_result.albums = Some(albums);
                }

                if let Some(squid_artists) = nested.artists {
                    let artists: Vec<DabArtist> = squid_artists
                        .items
                        .into_iter()
                        .map(|a| DabArtist {
                            id: a.id.to_string(),
                            name: a.name,
                            albums_count: None,
                            albums_as_primary_artist_count: None,
                            albums_as_primary_composer_count: None,
                            slug: None,
                            image: a.picture.map(|pic| format_artist_picture(&pic)),
                            biography: None,
                            similar_artist_ids: None,
                            information: None,
                        })
                        .collect();
                    search_result.artists = Some(artists);
                }
            }
        }

        Ok(search_result)
    }

    /// Get stream URL for a track
    pub async fn get_stream_url(&self, track_id: &str) -> DabResult<String> {
        debug!("get_stream_url called for track_id: {}", track_id);

        // First, check if we have this track cached locally
        if let Some(ref cache) = self.cache {
            debug!("Cache is available, cloning and checking for track {}", track_id);

            // Clone cache to avoid borrowing issues
            let mut cache_clone = cache.clone();
            debug!("Cache cloned, calling get_track_path for track {}", track_id);

            match cache_clone.get_track_path(track_id).await {
                Ok(Some(cached_path)) => {
                    let file_url = format!("file://{}", cached_path.display());
                    info!("✓ Using cached file for track {}: {}", track_id, file_url);
                    return Ok(file_url);
                }
                Ok(None) => {
                    debug!("get_track_path returned None for track {}", track_id);
                }
                Err(e) => {
                    debug!("get_track_path error for track {}: {:?}", track_id, e);
                }
            }

            // Check if we have a cached stream URL that's still valid
            if let Some(cached_url) = cache.get_cached_url(track_id).await? {
                info!("Found cached URL for track {}, using it", track_id);
                return Ok(cached_url);
            } else {
                debug!("get_cached_url returned None for track {}", track_id);
            }
        } else {
            debug!("Cache is None, cannot check for cached track");
        }

        info!(
            "Track {} not found in cache, fetching stream URL from API",
            track_id
        );
        #[derive(Deserialize)]
        struct StreamResponse {
            url: String,
        }

        let body = self
            .fetch_text_with_fallback("Stream URL", |target| {
                format!(
                    "{}/track/?id={}&quality={}",
                    target.base_url,
                    track_id,
                    self.map_quality_to_param()
                )
            })
            .await?;

        let body_snippet = if body.len() > 512 {
            format!("{}…", &body[..512])
        } else {
            body.clone()
        };
        debug!("Stream URL raw response for {}: {}", track_id, body_snippet);

        // Try parsing as StreamResponse
        if let Ok(stream_response) = serde_json::from_str::<StreamResponse>(&body) {
            return Self::validate_stream_url(&stream_response.url);
        }

        // Try parsing as plain string
        if let Ok(url) = serde_json::from_str::<String>(&body) {
            let trimmed = url.trim();
            if !trimmed.is_empty() {
                return Self::validate_stream_url(trimmed);
            }
        }

        // Try extracting from JSON value
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
            if let Some(url) = Self::extract_stream_url(&value) {
                return Self::validate_stream_url(url);
            }
        }

        // Last resort: treat as plain text
        let trimmed = body.trim().trim_matches('"');
        if trimmed.is_empty() {
            Err(DabError::Network("Empty stream URL response".to_string()))
        } else {
            Self::validate_stream_url(trimmed)
        }
    }

    /// Get album information
    pub async fn get_album_info(&self, album_id: &str) -> DabResult<DabAlbum> {
        // First, check if we have this album in favorites
        if let Some(ref library) = self.library {
            let favorite_albums = library.get_favorite_albums();
            if let Some(fav_album) = favorite_albums.iter().find(|a| a.id == album_id) {
                info!("Found album {} in favorites cache", album_id);
                return Ok(DabAlbum {
                    id: fav_album.id.clone(),
                    title: fav_album.title.clone(),
                    artist: fav_album.artist.clone(),
                    artist_id: fav_album.artist_id.clone(),
                    release_date: fav_album.release_date.clone(),
                    genre: None,
                    cover: fav_album.cover.clone(),
                    tracks: None, // Will be populated from cache if available
                    track_count: fav_album.track_count,
                    duration: None,
                    label: None,
                    upc: None,
                    url: None,
                    streamable: Some(true),
                    downloadable: None,
                    media_count: None,
                    maximum_channel_count: None,
                    parental_warning: None,
                    popularity: None,
                    audio_quality: None,
                });
            }
        }

        // Check if we have cached tracks for this album_id
        if let Some(ref cache) = self.cache {
            // Try to find tracks with matching album_id in their metadata
            if let Ok(cached_files) = cache.get_all_cached_files().await {
                let mut album_tracks = Vec::new();
                let mut album_title = None;
                let mut album_artist = None;
                let mut album_cover = None;

                for (track_id, _path) in cached_files {
                    if let Ok(Some(metadata)) = cache.get_track_metadata(&track_id).await {
                        // Check if this track belongs to the album_id we're looking for
                        if metadata.album_id.as_deref() == Some(album_id) {
                            // This track belongs to our album
                            if album_title.is_none() {
                                album_title = metadata.album.clone();
                            }
                            if album_artist.is_none() {
                                album_artist = metadata.album_artist.clone().or_else(|| metadata.artist.clone());
                            }
                            if album_cover.is_none() {
                                album_cover = metadata.cover_url.clone();
                            }

                            // Add track to list
                            album_tracks.push(DabTrack {
                                id: track_id.clone(),
                                title: metadata.title.unwrap_or_else(|| "Unknown".to_string()),
                                artist: metadata.artist.unwrap_or_default(),
                                artist_id: metadata.artist_id.clone(),
                                album_title: metadata.album.clone(),
                                album_cover: metadata.cover_url.clone(),
                                album_id: Some(album_id.to_string()),
                                release_date: metadata.release_date.clone(),
                                genre: metadata.genre.clone(),
                                duration: metadata.duration_ms,
                                audio_quality: None, // ID3 audio_quality is String, DabTrack expects enum
                                version: None,
                                label: None,
                                label_id: None,
                                upc: None,
                                media_count: None,
                                parental_warning: None,
                                streamable: Some(true),
                                purchasable: None,
                                previewable: None,
                                genre_id: None,
                                genre_slug: None,
                                genre_color: None,
                                release_date_stream: None,
                                release_date_download: None,
                                maximum_channel_count: None,
                                images: None,
                                isrc: metadata.isrc.clone(),
                            });
                        }
                    }
                }

                // If we found cached tracks for this album, return album info from cache
                if !album_tracks.is_empty() {
                    info!(
                        "Found {} cached tracks for album {}",
                        album_tracks.len(),
                        album_id
                    );

                    // Sort tracks by track number
                    album_tracks.sort_by(|a, b| {
                        // Try to parse track IDs as numbers for sorting
                        let a_num = a.id.parse::<u64>().unwrap_or(0);
                        let b_num = b.id.parse::<u64>().unwrap_or(0);
                        a_num.cmp(&b_num)
                    });

                    return Ok(DabAlbum {
                        id: album_id.to_string(),
                        title: album_title.unwrap_or_else(|| "Unknown Album".to_string()),
                        artist: album_artist.unwrap_or_default(),
                        artist_id: album_tracks.first().and_then(|t| t.artist_id.clone()),
                        release_date: album_tracks.first().and_then(|t| t.release_date.clone()),
                        genre: album_tracks.first().and_then(|t| t.genre.clone()),
                        cover: album_cover,
                        tracks: Some(album_tracks.clone()),
                        track_count: Some(album_tracks.len() as u32),
                        duration: Some(album_tracks.iter().filter_map(|t| t.duration).sum()),
                        label: None,
                        upc: None,
                        url: None,
                        streamable: Some(true),
                        downloadable: None,
                        media_count: None,
                        maximum_channel_count: None,
                        parental_warning: None,
                        popularity: None,
                        audio_quality: None, // Will be None from cached tracks
                    });
                }
            }
        }

        // If not found in local cache, fetch from API
        info!(
            "Album {} not found in local cache, fetching from API",
            album_id
        );
        // Reuse common structures
        #[derive(Deserialize)]
        struct SquidAlbumInfo {
            id: i64,
            title: String,
            duration: Option<u32>,
            #[serde(rename = "numberOfTracks")]
            number_of_tracks: Option<u32>,
            #[serde(rename = "releaseDate")]
            release_date: Option<String>,
            cover: Option<String>,
            artist: Option<SquidAlbumArtist>,
            artists: Option<Vec<SquidAlbumArtist>>,
        }

        #[derive(Deserialize)]
        struct SquidAlbumArtist {
            id: i64,
            name: String,
            #[serde(default)]
            handle: Option<String>,
            #[serde(rename = "type", default)]
            artist_type: Option<String>,
            #[serde(default)]
            picture: Option<String>,
        }

        #[derive(Deserialize)]
        struct SquidTrackListItem {
            item: SquidTrackInfo,
        }

        #[derive(Deserialize)]
        struct SquidTrackInfo {
            id: i64,
            title: String,
            duration: Option<u32>,
            #[serde(rename = "trackNumber")]
            track_number: Option<u32>,
        }

        #[derive(Deserialize)]
        struct SquidTrackList {
            items: Vec<SquidTrackListItem>,
        }

        // Helper to format cover URLs
        fn format_cover_url(cover: &str) -> String {
            if cover.starts_with("http") {
                cover.to_string()
            } else {
                format!(
                    "https://resources.tidal.com/images/{}/1280x1280.jpg",
                    cover.replace('-', "/")
                )
            }
        }

        // The response is an array: [album_info, track_list]
        let response: serde_json::Value = self
            .fetch_with_fallback("Album Info", |target| {
                format!("{}/album/?id={}", target.base_url, album_id)
            })
            .await?;

        // Parse the array response
        let arr = response
            .as_array()
            .ok_or_else(|| DabError::Decode("Invalid album response format".to_string()))?;

        if arr.is_empty() {
            return Err(DabError::FileNotFound(format!(
                "Album {} not found",
                album_id
            )));
        }

        let album_info: SquidAlbumInfo =
            serde_json::from_value(arr[0].clone()).map_err(|e| DabError::Serialization(e))?;

        // Extract artist info - try singular artist first, then fallback to artists array
        let (album_artist_name, album_artist_id) = if let Some(artist) = &album_info.artist {
            (artist.name.clone(), Some(artist.id.to_string()))
        } else if let Some(artists) = &album_info.artists {
            if let Some(first_artist) = artists.first() {
                (first_artist.name.clone(), Some(first_artist.id.to_string()))
            } else {
                (String::new(), None)
            }
        } else {
            (String::new(), None)
        };

        // Parse tracks if available
        let mut tracks = None;
        if arr.len() > 1 {
            // Check if the second element is an error response
            if arr[1].get("status").is_some() {
                // This is an error response, skip track parsing
                debug!("Track list unavailable - API returned error status");
            } else {
                // Try to parse as track list
                if let Ok(track_list) = serde_json::from_value::<SquidTrackList>(arr[1].clone()) {
                    tracks = Some(
                        track_list
                            .items
                            .into_iter()
                            .map(|item| DabTrack {
                                id: item.item.id.to_string(),
                                title: item.item.title,
                                artist: album_artist_name.clone(),
                                artist_id: album_artist_id.clone(),
                                album_title: Some(album_info.title.clone()),
                                album_cover: album_info.cover.as_ref().map(|c| format_cover_url(c)),
                                album_id: Some(album_id.to_string()),
                                release_date: None,
                                genre: None,
                                duration: item.item.duration,
                                audio_quality: None,
                                version: None,
                                label: None,
                                label_id: None,
                                upc: None,
                                media_count: None,
                                parental_warning: None,
                                streamable: Some(true),
                                purchasable: None,
                                previewable: None,
                                genre_id: None,
                                genre_slug: None,
                                genre_color: None,
                                release_date_stream: None,
                                release_date_download: None,
                                maximum_channel_count: None,
                                images: None,
                                isrc: None,
                            })
                            .collect(),
                    );
                }
            }
        }

        Ok(DabAlbum {
            id: album_info.id.to_string(),
            title: album_info.title,
            artist: album_artist_name,
            artist_id: album_artist_id,
            release_date: album_info.release_date,
            genre: None,
            cover: album_info.cover.map(|c| format_cover_url(&c)),
            tracks,
            track_count: album_info.number_of_tracks,
            duration: album_info.duration,
            label: None,
            upc: None,
            url: None,
            streamable: Some(true),
            downloadable: None,
            media_count: None,
            maximum_channel_count: None,
            parental_warning: None,
            popularity: None,
            audio_quality: None,
        })
    }

    /// Get artist discography
    /// Now uses /artist/?f= endpoint instead of /discography/?artistId=
    pub async fn get_artist_discography(
        &self,
        artist_id: &str,
    ) -> DabResult<(DabArtist, Vec<DabAlbum>)> {
        // Use get_artist_albums to fetch albums via /artist/?f= endpoint
        let albums = self.get_artist_albums(artist_id).await?;

        // Create a basic DabArtist from the artist_id
        // Note: We don't have full artist details from /artist/?f= endpoint
        // This is a simplified version that provides basic artist info
        let artist = DabArtist {
            id: artist_id.to_string(),
            name: albums
                .first()
                .map(|a| a.artist.clone())
                .unwrap_or_else(|| "Unknown Artist".to_string()),
            albums_count: Some(albums.len() as u32),
            albums_as_primary_artist_count: Some(albums.len() as u32),
            albums_as_primary_composer_count: None,
            slug: None,
            image: None,
            biography: None,
            similar_artist_ids: None,
            information: None,
        };

        Ok((artist, albums))
    }

    /// Get artist albums using the /artist/?f= endpoint
    /// This endpoint returns a paginated list of albums for the artist
    pub async fn get_artist_albums(&self, artist_id: &str) -> DabResult<Vec<DabAlbum>> {
        // Response structure for /artist/?f= endpoint
        #[derive(Deserialize)]
        struct ArtistPageResponse {
            rows: Vec<ArtistPageRow>,
        }

        #[derive(Deserialize)]
        struct ArtistPageRow {
            modules: Vec<ArtistPageModule>,
        }

        #[derive(Deserialize)]
        struct ArtistPageModule {
            #[serde(rename = "pagedList")]
            paged_list: Option<PagedAlbumList>,
        }

        #[derive(Deserialize)]
        struct PagedAlbumList {
            items: Vec<TidalAlbum>,
        }

        #[derive(Deserialize)]
        struct TidalAlbum {
            id: i64,
            title: String,
            cover: Option<String>,
            artists: Option<Vec<TidalArtist>>,
            #[serde(rename = "numberOfTracks")]
            number_of_tracks: Option<u32>,
            #[serde(rename = "releaseDate")]
            release_date: Option<String>,
            duration: Option<u32>,
            #[serde(rename = "allowStreaming")]
            allow_streaming: Option<bool>,
        }

        #[derive(Deserialize)]
        struct TidalArtist {
            id: i64,
            name: String,
        }

        // Helper function to format cover URLs
        fn format_cover_url(cover: &str) -> String {
            if cover.starts_with("http") {
                cover.to_string()
            } else {
                format!(
                    "https://resources.tidal.com/images/{}/1280x1280.jpg",
                    cover.replace('-', "/")
                )
            }
        }

        // Fetch artist albums using the /artist/?f= endpoint
        let response: serde_json::Value = self
            .fetch_with_fallback("Artist Albums", |target| {
                format!("{}/artist/?f={}", target.base_url, artist_id)
            })
            .await?;

        // The response is an array with one element containing the page structure
        let response_array = response
            .as_array()
            .ok_or_else(|| DabError::Decode("Invalid artist albums response format".to_string()))?;

        if response_array.is_empty() {
            return Ok(Vec::new());
        }

        // Parse the first element as ArtistPageResponse
        let page_response: ArtistPageResponse = serde_json::from_value(response_array[0].clone())
            .map_err(|e| DabError::Serialization(e))?;

        // Extract albums from the page structure
        let mut all_albums = Vec::new();
        for row in page_response.rows {
            for module in row.modules {
                if let Some(paged_list) = module.paged_list {
                    for tidal_album in paged_list.items {
                        // Extract artist info from the artists array
                        let (album_artist_name, album_artist_id) =
                            if let Some(artists) = &tidal_album.artists {
                                if let Some(first_artist) = artists.first() {
                                    (first_artist.name.clone(), Some(first_artist.id.to_string()))
                                } else {
                                    (String::new(), Some(artist_id.to_string()))
                                }
                            } else {
                                (String::new(), Some(artist_id.to_string()))
                            };

                        let album = DabAlbum {
                            id: tidal_album.id.to_string(),
                            title: tidal_album.title,
                            artist: album_artist_name,
                            artist_id: album_artist_id,
                            release_date: tidal_album.release_date,
                            genre: None,
                            cover: tidal_album.cover.map(|c| format_cover_url(&c)),
                            tracks: None,
                            track_count: tidal_album.number_of_tracks,
                            duration: tidal_album.duration,
                            label: None,
                            upc: None,
                            url: None,
                            streamable: tidal_album.allow_streaming,
                            downloadable: None,
                            media_count: None,
                            maximum_channel_count: None,
                            parental_warning: None,
                            popularity: None,
                            audio_quality: None,
                        };

                        all_albums.push(album);
                    }
                }
            }
        }

        Ok(all_albums)
    }
}
