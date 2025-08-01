use log::{debug, error, info, warn};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

use crate::api_cache::{ApiCache, CachedResponse};
use crate::error::{DabError, DabResult};
use crate::search::{DabAlbum, DabArtist, SearchResult};

/// Unified async client for all network operations
#[derive(Clone)]
pub struct AsyncClient {
    client: Client,
    base_url: String,
    cache: Arc<ApiCache>,
}

/// Request types for different API operations
#[derive(Debug, Clone)]
pub enum ApiRequest {
    Search {
        query: String,
        search_type: String,
        limit: u32,
    },
    GetAlbum {
        album_id: String,
    },
    GetArtistDiscography {
        artist_id: String,
    },
    GetStreamUrl {
        track_id: String,
        quality: Option<String>,
    },
    GetLyrics {
        artist: String,
        title: String,
    },
}

/// Response types for different API operations
#[derive(Debug, Clone)]
pub enum ApiResponse {
    SearchResult(SearchResult),
    AlbumInfo(DabAlbum),
    ArtistDiscography {
        artist: DabArtist,
        albums: Vec<DabAlbum>,
    },
    StreamUrl(String),
    Lyrics {
        lyrics: String,
        unsynced: bool,
    },
    Error(String),
}

/// Command for background network task
#[derive(Debug)]
pub struct NetworkCommand {
    pub request: ApiRequest,
    pub response_tx: oneshot::Sender<ApiResponse>,
}

/// Background network task manager
pub struct NetworkManager {
    client: AsyncClient,
    command_rx: mpsc::UnboundedReceiver<NetworkCommand>,
}

impl AsyncClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            base_url: "https://dab.yeet.su/api".to_string(),
            cache: Arc::new(ApiCache::new()),
        }
    }

    pub fn new_with_base_url(base_url: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
            cache: Arc::new(ApiCache::new()),
        }
    }

    pub fn new_with_cache(cache: Arc<ApiCache>) -> Self {
        Self {
            client: Client::new(),
            base_url: "https://dab.yeet.su/api".to_string(),
            cache,
        }
    }

    /// Execute a network request asynchronously
    pub async fn execute_request(&self, request: ApiRequest) -> ApiResponse {
        match request {
            ApiRequest::Search {
                query,
                search_type,
                limit,
            } => self.handle_search_request(query, search_type, limit).await,
            ApiRequest::GetAlbum { album_id } => self.handle_album_request(album_id).await,
            ApiRequest::GetArtistDiscography { artist_id } => {
                self.handle_discography_request(artist_id).await
            }
            ApiRequest::GetStreamUrl { track_id, quality } => {
                self.handle_stream_url_request(track_id, quality).await
            }
            ApiRequest::GetLyrics { artist, title } => {
                self.handle_lyrics_request(artist, title).await
            }
        }
    }

    async fn handle_search_request(
        &self,
        query: String,
        search_type: String,
        limit: u32,
    ) -> ApiResponse {
        let url = format!("{}/search?q={}&type={}&limit={}", 
            self.base_url, 
            urlencoding::encode(&query), 
            search_type, 
            limit
        );

        // Check cache first (excluding stream URL requests from cache)
        if let Some(cached_response) = self.cache.get(&url).await {
            if let CachedResponse::SearchResult(search_result) = cached_response {
                debug!("Returning cached search result for query: {}", query);
                return ApiResponse::SearchResult(search_result);
            }
        }

        // Make HTTP request
        let mut request_builder = self.client.get(&url);
        
        // Add conditional request headers if cache entry exists
        if let Some((header_name, header_value)) = self.cache.should_revalidate(&url).await {
            request_builder = request_builder.header(&header_name, header_value);
        }

        match request_builder.send().await {
            Ok(response) => {
                let headers = response.headers().clone();
                let status = response.status();

                if status == reqwest::StatusCode::NOT_MODIFIED {
                    // 304 Not Modified - update cache and return cached response
                    self.cache.update_on_not_modified(&url, &headers).await;
                    if let Some(cached_response) = self.cache.get(&url).await {
                        if let CachedResponse::SearchResult(search_result) = cached_response {
                            debug!("Returning revalidated cached search result for query: {}", query);
                            return ApiResponse::SearchResult(search_result);
                        }
                    }
                }

                match response.json::<SearchResult>().await {
                    Ok(search_result) => {
                        debug!("Search request successful for query: {}", query);
                        
                        // Cache the response
                        self.cache.put(
                            url,
                            CachedResponse::SearchResult(search_result.clone()),
                            &headers,
                        ).await;
                        
                        ApiResponse::SearchResult(search_result)
                    }
                    Err(e) => {
                        error!("Failed to parse search response: {}", e);
                        ApiResponse::Error(format!("Failed to parse search response: {}", e))
                    }
                }
            }
            Err(e) => {
                error!("Search request failed: {}", e);
                ApiResponse::Error(format!("Search request failed: {}", e))
            }
        }
    }

    async fn handle_album_request(&self, album_id: String) -> ApiResponse {
        let url = format!("{}/album?albumId={}", self.base_url, urlencoding::encode(&album_id));

        // Check cache first
        if let Some(cached_response) = self.cache.get(&url).await {
            if let CachedResponse::AlbumInfo(album) = cached_response {
                debug!("Returning cached album info for ID: {}", album_id);
                return ApiResponse::AlbumInfo(album);
            }
        }

        // Make HTTP request
        let mut request_builder = self.client.get(&url);
        
        // Add conditional request headers if cache entry exists
        if let Some((header_name, header_value)) = self.cache.should_revalidate(&url).await {
            request_builder = request_builder.header(&header_name, header_value);
        }

        match request_builder.send().await {
            Ok(response) => {
                let headers = response.headers().clone();
                let status = response.status();

                if status == reqwest::StatusCode::NOT_MODIFIED {
                    // 304 Not Modified - update cache and return cached response
                    self.cache.update_on_not_modified(&url, &headers).await;
                    if let Some(cached_response) = self.cache.get(&url).await {
                        if let CachedResponse::AlbumInfo(album) = cached_response {
                            debug!("Returning revalidated cached album info for ID: {}", album_id);
                            return ApiResponse::AlbumInfo(album);
                        }
                    }
                }

                #[derive(Deserialize)]
                struct AlbumResponse {
                    album: DabAlbum,
                }

                match response.json::<AlbumResponse>().await {
                    Ok(album_response) => {
                        debug!("Album request successful for ID: {}", album_id);
                        
                        // Cache the response
                        self.cache.put(
                            url,
                            CachedResponse::AlbumInfo(album_response.album.clone()),
                            &headers,
                        ).await;
                        
                        ApiResponse::AlbumInfo(album_response.album)
                    }
                    Err(e) => {
                        error!("Failed to parse album response: {}", e);
                        ApiResponse::Error(format!("Failed to parse album response: {}", e))
                    }
                }
            }
            Err(e) => {
                error!("Album request failed: {}", e);
                ApiResponse::Error(format!("Album request failed: {}", e))
            }
        }
    }

    async fn handle_discography_request(&self, artist_id: String) -> ApiResponse {
        let url = format!("{}/discography?artistId={}", self.base_url, urlencoding::encode(&artist_id));

        // Check cache first
        if let Some(cached_response) = self.cache.get(&url).await {
            if let CachedResponse::ArtistDiscography { artist, albums } = cached_response {
                debug!("Returning cached discography for artist ID: {}", artist_id);
                return ApiResponse::ArtistDiscography { artist, albums };
            }
        }

        // Make HTTP request
        let mut request_builder = self.client.get(&url);
        
        // Add conditional request headers if cache entry exists
        if let Some((header_name, header_value)) = self.cache.should_revalidate(&url).await {
            request_builder = request_builder.header(&header_name, header_value);
        }

        match request_builder.send().await {
            Ok(response) => {
                let headers = response.headers().clone();
                let status = response.status();

                if status == reqwest::StatusCode::NOT_MODIFIED {
                    // 304 Not Modified - update cache and return cached response
                    self.cache.update_on_not_modified(&url, &headers).await;
                    if let Some(cached_response) = self.cache.get(&url).await {
                        if let CachedResponse::ArtistDiscography { artist, albums } = cached_response {
                            debug!("Returning revalidated cached discography for artist ID: {}", artist_id);
                            return ApiResponse::ArtistDiscography { artist, albums };
                        }
                    }
                }

                #[derive(Deserialize)]
                struct DiscographyResponse {
                    artist: DabArtist,
                    albums: Vec<DabAlbum>,
                }

                match response.json::<DiscographyResponse>().await {
                    Ok(discog_response) => {
                        debug!("Discography request successful for artist ID: {}", artist_id);
                        
                        // Cache the response
                        self.cache.put(
                            url,
                            CachedResponse::ArtistDiscography {
                                artist: discog_response.artist.clone(),
                                albums: discog_response.albums.clone(),
                            },
                            &headers,
                        ).await;
                        
                        ApiResponse::ArtistDiscography {
                            artist: discog_response.artist,
                            albums: discog_response.albums,
                        }
                    }
                    Err(e) => {
                        error!("Failed to parse discography response: {}", e);
                        ApiResponse::Error(format!("Failed to parse discography response: {}", e))
                    }
                }
            }
            Err(e) => {
                error!("Discography request failed: {}", e);
                ApiResponse::Error(format!("Discography request failed: {}", e))
            }
        }
    }

    async fn handle_stream_url_request(
        &self,
        track_id: String,
        quality: Option<String>,
    ) -> ApiResponse {
        let url = format!("{}/stream", self.base_url);
        let mut params = vec![("trackId", track_id.as_str())];

        if let Some(ref q) = quality {
            params.push(("quality", q.as_str()));
        }

        match self.client.get(&url).query(&params).send().await {
            Ok(response) => {
                #[derive(Deserialize)]
                struct StreamResponse {
                    url: String,
                }

                match response.json::<StreamResponse>().await {
                    Ok(stream_response) => {
                        debug!("Stream URL request successful for track ID: {}", track_id);
                        ApiResponse::StreamUrl(stream_response.url)
                    }
                    Err(e) => {
                        error!("Failed to parse stream URL response: {}", e);
                        ApiResponse::Error(format!("Failed to parse stream URL response: {}", e))
                    }
                }
            }
            Err(e) => {
                error!("Stream URL request failed: {}", e);
                ApiResponse::Error(format!("Stream URL request failed: {}", e))
            }
        }
    }

    async fn handle_lyrics_request(&self, artist: String, title: String) -> ApiResponse {
        let url = format!("{}/lyrics?artist={}&title={}", 
            self.base_url, 
            urlencoding::encode(&artist), 
            urlencoding::encode(&title)
        );

        // Check cache first
        if let Some(cached_response) = self.cache.get(&url).await {
            if let CachedResponse::Lyrics { lyrics, unsynced } = cached_response {
                debug!("Returning cached lyrics for {}: {}", artist, title);
                return ApiResponse::Lyrics { lyrics, unsynced };
            }
        }

        // Make HTTP request
        let mut request_builder = self.client.get(&url);
        
        // Add conditional request headers if cache entry exists
        if let Some((header_name, header_value)) = self.cache.should_revalidate(&url).await {
            request_builder = request_builder.header(&header_name, header_value);
        }

        match request_builder.send().await {
            Ok(response) => {
                let headers = response.headers().clone();
                let status = response.status();

                if status == reqwest::StatusCode::NOT_MODIFIED {
                    // 304 Not Modified - update cache and return cached response
                    self.cache.update_on_not_modified(&url, &headers).await;
                    if let Some(cached_response) = self.cache.get(&url).await {
                        if let CachedResponse::Lyrics { lyrics, unsynced } = cached_response {
                            debug!("Returning revalidated cached lyrics for {}: {}", artist, title);
                            return ApiResponse::Lyrics { lyrics, unsynced };
                        }
                    }
                }

                #[derive(Deserialize)]
                struct LyricsResponse {
                    lyrics: String,
                    unsynced: Option<bool>,
                }

                match response.json::<LyricsResponse>().await {
                    Ok(lyrics_response) => {
                        debug!("Lyrics request successful for {}: {}", artist, title);
                        
                        let lyrics_data = ApiResponse::Lyrics {
                            lyrics: lyrics_response.lyrics,
                            unsynced: lyrics_response.unsynced.unwrap_or(true),
                        };
                        
                        // Cache the response
                        if let ApiResponse::Lyrics { lyrics, unsynced } = &lyrics_data {
                            self.cache.put(
                                url,
                                CachedResponse::Lyrics {
                                    lyrics: lyrics.clone(),
                                    unsynced: *unsynced,
                                },
                                &headers,
                            ).await;
                        }
                        
                        lyrics_data
                    }
                    Err(e) => {
                        error!("Failed to parse lyrics response: {}", e);
                        ApiResponse::Error(format!("Failed to parse lyrics response: {}", e))
                    }
                }
            }
            Err(e) => {
                error!("Lyrics request failed: {}", e);
                ApiResponse::Error(format!("Lyrics request failed: {}", e))
            }
        }
    }
}

impl NetworkManager {
    pub fn new(client: AsyncClient) -> (Self, mpsc::UnboundedSender<NetworkCommand>) {
        let (command_tx, command_rx) = mpsc::unbounded_channel();

        let manager = Self { client, command_rx };

        (manager, command_tx)
    }

    /// Start the background network task
    pub async fn run(mut self) {
        info!("Network manager started");

        while let Some(command) = self.command_rx.recv().await {
            let client = self.client.clone();

            // Spawn each request in its own task to handle them concurrently
            tokio::spawn(async move {
                let response = client.execute_request(command.request).await;

                if let Err(e) = command.response_tx.send(response) {
                    warn!("Failed to send network response back to caller: {:?}", e);
                }
            });
        }

        info!("Network manager stopped");
    }
}

/// High-level async client interface for easy use
#[derive(Clone)]
pub struct AsyncNetworkClient {
    command_tx: mpsc::UnboundedSender<NetworkCommand>,
}

impl AsyncNetworkClient {
    pub fn new(command_tx: mpsc::UnboundedSender<NetworkCommand>) -> Self {
        Self { command_tx }
    }

    /// Perform search request asynchronously
    pub async fn search(
        &self,
        query: String,
        search_type: String,
        limit: u32,
    ) -> DabResult<SearchResult> {
        let (response_tx, response_rx) = oneshot::channel();

        let command = NetworkCommand {
            request: ApiRequest::Search {
                query,
                search_type,
                limit,
            },
            response_tx,
        };

        self.command_tx
            .send(command)
            .map_err(|e| DabError::Network(format!("Failed to send search command: {}", e)))?;

        match response_rx.await {
            Ok(ApiResponse::SearchResult(result)) => Ok(result),
            Ok(ApiResponse::Error(err)) => Err(DabError::Network(err)),
            Ok(_) => Err(DabError::Network(
                "Unexpected response type for search".to_string(),
            )),
            Err(e) => Err(DabError::Network(format!(
                "Failed to receive search response: {}",
                e
            ))),
        }
    }

    /// Get album information asynchronously
    pub async fn get_album(&self, album_id: String) -> DabResult<DabAlbum> {
        let (response_tx, response_rx) = oneshot::channel();

        let command = NetworkCommand {
            request: ApiRequest::GetAlbum { album_id },
            response_tx,
        };

        self.command_tx
            .send(command)
            .map_err(|e| DabError::Network(format!("Failed to send album command: {}", e)))?;

        match response_rx.await {
            Ok(ApiResponse::AlbumInfo(album)) => Ok(album),
            Ok(ApiResponse::Error(err)) => Err(DabError::Network(err)),
            Ok(_) => Err(DabError::Network(
                "Unexpected response type for album".to_string(),
            )),
            Err(e) => Err(DabError::Network(format!(
                "Failed to receive album response: {}",
                e
            ))),
        }
    }

    /// Get artist discography asynchronously
    pub async fn get_artist_discography(
        &self,
        artist_id: String,
    ) -> DabResult<(DabArtist, Vec<DabAlbum>)> {
        let (response_tx, response_rx) = oneshot::channel();

        let command = NetworkCommand {
            request: ApiRequest::GetArtistDiscography { artist_id },
            response_tx,
        };

        self.command_tx
            .send(command)
            .map_err(|e| DabError::Network(format!("Failed to send discography command: {}", e)))?;

        match response_rx.await {
            Ok(ApiResponse::ArtistDiscography { artist, albums }) => Ok((artist, albums)),
            Ok(ApiResponse::Error(err)) => Err(DabError::Network(err)),
            Ok(_) => Err(DabError::Network(
                "Unexpected response type for discography".to_string(),
            )),
            Err(e) => Err(DabError::Network(format!(
                "Failed to receive discography response: {}",
                e
            ))),
        }
    }

    /// Get stream URL asynchronously
    pub async fn get_stream_url(
        &self,
        track_id: String,
        quality: Option<String>,
    ) -> DabResult<String> {
        let (response_tx, response_rx) = oneshot::channel();

        let command = NetworkCommand {
            request: ApiRequest::GetStreamUrl { track_id, quality },
            response_tx,
        };

        self.command_tx
            .send(command)
            .map_err(|e| DabError::Network(format!("Failed to send stream URL command: {}", e)))?;

        match response_rx.await {
            Ok(ApiResponse::StreamUrl(url)) => Ok(url),
            Ok(ApiResponse::Error(err)) => Err(DabError::Network(err)),
            Ok(_) => Err(DabError::Network(
                "Unexpected response type for stream URL".to_string(),
            )),
            Err(e) => Err(DabError::Network(format!(
                "Failed to receive stream URL response: {}",
                e
            ))),
        }
    }

    /// Get lyrics asynchronously
    pub async fn get_lyrics(&self, artist: String, title: String) -> DabResult<(String, bool)> {
        let (response_tx, response_rx) = oneshot::channel();

        let command = NetworkCommand {
            request: ApiRequest::GetLyrics { artist, title },
            response_tx,
        };

        self.command_tx
            .send(command)
            .map_err(|e| DabError::Network(format!("Failed to send lyrics command: {}", e)))?;

        match response_rx.await {
            Ok(ApiResponse::Lyrics { lyrics, unsynced }) => Ok((lyrics, unsynced)),
            Ok(ApiResponse::Error(err)) => Err(DabError::Network(err)),
            Ok(_) => Err(DabError::Network(
                "Unexpected response type for lyrics".to_string(),
            )),
            Err(e) => Err(DabError::Network(format!(
                "Failed to receive lyrics response: {}",
                e
            ))),
        }
    }
}
