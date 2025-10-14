use log::{debug, error, info, warn};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

use crate::api_cache::{ApiCache, CachedResponse};
use crate::config::Config;
use crate::error::{DabError, DabResult};
use crate::music_provider::MusicProviderClient;
use crate::search::{DabAlbum, DabArtist, SearchResult};

/// Unified async client for all network operations
#[derive(Clone)]
pub struct AsyncClient {
    provider: MusicProviderClient,
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
            provider: MusicProviderClient::new(),
            cache: Arc::new(ApiCache::new()),
        }
    }

    pub fn new_with_cache(cache: Arc<ApiCache>) -> Self {
        Self {
            provider: MusicProviderClient::new(),
            cache,
        }
    }

    pub fn new_with_config(config: &Config) -> Self {
        Self {
            provider: MusicProviderClient::new_with_config(config.clone()),
            cache: Arc::new(ApiCache::new()),
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
        // Create a cache key for the search
        let cache_key = format!("search:{}:{}:{}", search_type, query, limit);

        // Check cache first
        if let Some(cached_response) = self.cache.get(&cache_key).await {
            if let CachedResponse::SearchResult(search_result) = cached_response {
                debug!("Returning cached search result for query: {}", query);
                return ApiResponse::SearchResult(search_result);
            }
        }

        // Use music provider to perform search
        match self.provider.search(&query, &search_type, limit).await {
            Ok(search_result) => {
                debug!("Search request successful for query: {}", query);

                // Cache the response
                self.cache
                    .put(
                        cache_key,
                        CachedResponse::SearchResult(search_result.clone()),
                        &Default::default(), // Empty headers for backend responses
                    )
                    .await;

                ApiResponse::SearchResult(search_result)
            }
            Err(e) => {
                error!("Search request failed: {}", e);
                ApiResponse::Error(format!("Search request failed: {}", e))
            }
        }
    }

    async fn handle_album_request(&self, album_id: String) -> ApiResponse {
        let cache_key = format!("album:{}", album_id);

        // Check cache first
        if let Some(cached_response) = self.cache.get(&cache_key).await {
            if let CachedResponse::AlbumInfo(album) = cached_response {
                debug!("Returning cached album info for ID: {}", album_id);
                return ApiResponse::AlbumInfo(album);
            }
        }

        // Use music provider to get album info
        match self.provider.get_album_info(&album_id).await {
            Ok(album) => {
                debug!("Album request successful for ID: {}", album_id);

                // Cache the response
                self.cache
                    .put(
                        cache_key,
                        CachedResponse::AlbumInfo(album.clone()),
                        &Default::default(),
                    )
                    .await;

                ApiResponse::AlbumInfo(album)
            }
            Err(e) => {
                error!("Album request failed: {}", e);
                ApiResponse::Error(format!("Album request failed: {}", e))
            }
        }
    }

    async fn handle_discography_request(&self, artist_id: String) -> ApiResponse {
        let cache_key = format!("discography:{}", artist_id);

        // Check cache first
        if let Some(cached_response) = self.cache.get(&cache_key).await {
            if let CachedResponse::ArtistDiscography { artist, albums } = cached_response {
                debug!("Returning cached discography for artist ID: {}", artist_id);
                return ApiResponse::ArtistDiscography { artist, albums };
            }
        }

        // Use music provider to get artist discography
        match self.provider.get_artist_discography(&artist_id).await {
            Ok((artist, albums)) => {
                debug!(
                    "Discography request successful for artist ID: {}",
                    artist_id
                );

                // Cache the response
                self.cache
                    .put(
                        cache_key,
                        CachedResponse::ArtistDiscography {
                            artist: artist.clone(),
                            albums: albums.clone(),
                        },
                        &Default::default(),
                    )
                    .await;

                ApiResponse::ArtistDiscography { artist, albums }
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
        _quality: Option<String>,
    ) -> ApiResponse {
        // Stream URLs should not be cached as they expire
        match self.provider.get_stream_url(&track_id).await {
            Ok(stream_url) => {
                debug!("Stream URL request successful for track ID: {}", track_id);
                ApiResponse::StreamUrl(stream_url)
            }
            Err(e) => {
                error!("Stream URL request failed: {}", e);
                ApiResponse::Error(format!("Stream URL request failed: {}", e))
            }
        }
    }

    async fn handle_lyrics_request(&self, artist: String, title: String) -> ApiResponse {
        let cache_key = format!("lyrics:{}:{}", artist, title);

        // Check cache first
        if let Some(cached_response) = self.cache.get(&cache_key).await {
            if let CachedResponse::Lyrics { lyrics, unsynced } = cached_response {
                debug!("Returning cached lyrics for {}: {}", artist, title);
                return ApiResponse::Lyrics { lyrics, unsynced };
            }
        }

        // TODO: Add lyrics support to backend
        // For now, return not found
        ApiResponse::Lyrics {
            lyrics: "Lyrics not available".to_string(),
            unsynced: true,
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
