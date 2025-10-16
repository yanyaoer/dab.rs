use log::{debug, info, warn};
use reqwest::Client;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use super::decoder::ReadSeek;
use super::download_manager::{AsyncDownloadManager, DownloadEvent};
use super::streaming::StreamingAudioSource;
use super::Track;
use crate::cache::Cache;
use crate::error::{DabError, DabResult};

#[derive(Clone)]
pub struct AudioLoader {
    http_client: Client,
    cache: Arc<tokio::sync::RwLock<Cache>>,
    download_manager: Arc<AsyncDownloadManager>,
}

/// Result of loading a track - can be either traditional seekable or streaming
pub enum LoadResult {
    Seekable(Box<dyn ReadSeek>),
    Streaming {
        source: Arc<StreamingAudioSource>,
        events: mpsc::UnboundedReceiver<DownloadEvent>,
    },
    /// Simple in-memory buffer for download-complete mode (like test_squid_direct)
    InMemory(Vec<u8>),
}

impl AudioLoader {
    pub async fn new(cache: Cache) -> DabResult<Self> {
        let download_manager = Arc::new(AsyncDownloadManager::new(cache.clone()).await?);

        Ok(Self {
            http_client: Client::new(),
            cache: Arc::new(tokio::sync::RwLock::new(cache)),
            download_manager,
        })
    }

    /// Load track for seekable access (traditional method - downloads fully first)
    pub async fn load_track_seekable(&self, track: &Track) -> DabResult<Box<dyn ReadSeek>> {
        // First check if we have a local cached version
        if let Some(cached_path) = self.cache.write().await.get_track_path(&track.id).await? {
            info!("Loading track from cache: {}", cached_path.display());
            return Ok(Box::new(std::fs::File::open(cached_path)?));
        }

        // Check if it's a local file
        let track_url = if track.is_local() {
            track.local_path.as_ref().unwrap()
        } else {
            // This shouldn't happen since PlayerEngine now handles URL fetching
            return Err(DabError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Track missing URL - PlayerEngine should have resolved this",
            )));
        };

        if track_url.starts_with('/') || track_url.starts_with("file://") {
            let path = track_url.strip_prefix("file://").unwrap_or(track_url);
            info!("Loading local track: {}", path);
            return Ok(Box::new(std::fs::File::open(path)?));
        }

        // For remote files, we need to download first to enable seeking
        info!("Downloading track for seekable access: {}", track_url);
        self.download_and_cache_track_url(track, track_url).await
    }

    /// Load track for streaming playback (new method - starts playback while downloading)
    pub async fn load_track_streaming(
        &self,
        track: &Track,
        stream_url: &str,
    ) -> DabResult<LoadResult> {
        // First check if we have a local cached version
        if let Some(cached_path) = self.cache.write().await.get_track_path(&track.id).await? {
            info!("Loading track from cache: {}", cached_path.display());
            info!(
                "DOWNLOAD-COMPLETE: Returning LoadResult::Seekable for cached track {}",
                track.id
            );
            return Ok(LoadResult::Seekable(Box::new(std::fs::File::open(
                cached_path,
            )?)));
        }

        // Check if it's a local file
        if track.is_local() {
            let track_url = track.local_path.as_ref().unwrap();
            if track_url.starts_with('/') || track_url.starts_with("file://") {
                let path = track_url.strip_prefix("file://").unwrap_or(track_url);
                info!("Loading local track: {}", path);
                info!("DOWNLOAD-COMPLETE: Returning LoadResult::Seekable for local track");
                return Ok(LoadResult::Seekable(Box::new(std::fs::File::open(path)?)));
            }
        }

        // Check if we're in download-complete mode (streaming_buffer == 0)
        let config = crate::config::Config::load();
        info!(
            "Current streaming_buffer setting: {} MB",
            config.streaming_buffer
        );
        if config.streaming_buffer == 0 {
            info!("DOWNLOAD-COMPLETE MODE ACTIVATED: downloading entire file before playback for track {}", track.id);

            // Download entire file into memory (like test_squid_direct)
            let audio_data = self.download_to_memory(track, stream_url).await?;

            info!(
                "Download complete for track {} ({} bytes), returning in-memory data",
                track.id,
                audio_data.len()
            );
            return Ok(LoadResult::InMemory(audio_data));
        }

        // Check if already downloading or available in download manager
        if let Some(existing_source) = self.download_manager.get_streaming_source(&track.id).await {
            info!("Using existing streaming source for track: {}", track.id);
            return Ok(LoadResult::Streaming {
                source: existing_source,
                events: mpsc::unbounded_channel().1, // Empty event channel
            });
        }

        // Start streaming download
        info!("Starting streaming download for track: {}", track.id);
        let (streaming_source, events) = self
            .download_manager
            .start_download(track.clone(), stream_url.to_string())
            .await?;

        Ok(LoadResult::Streaming {
            source: streaming_source,
            events,
        })
    }

    /// Get download progress for a track (0.0 to 1.0)
    pub async fn get_download_progress(&self, track_id: &str) -> Option<f32> {
        self.download_manager.get_progress(track_id).await
    }

    /// Check if track is ready for streaming
    pub async fn is_stream_ready(&self, track_id: &str) -> bool {
        self.download_manager.is_stream_ready(track_id).await
    }

    /// Cancel download for a track
    pub async fn cancel_download(&self, track_id: &str) -> DabResult<()> {
        self.download_manager.cancel_download(track_id).await
    }

    /// Download entire file into memory (like test_squid_direct)
    async fn download_to_memory(&self, track: &Track, url: &str) -> DabResult<Vec<u8>> {
        info!(
            "Downloading entire file to memory for track: {}",
            track.title
        );

        let response = self.http_client.get(url).send().await?.error_for_status()?;
        let content_length = response.content_length().unwrap_or(0);

        info!("Content-Length: {} bytes", content_length);

        // Pre-allocate buffer if we know the size
        let mut audio_data = if content_length > 0 {
            Vec::with_capacity(content_length as usize)
        } else {
            Vec::new()
        };

        let mut stream = response.bytes_stream();
        let mut downloaded = 0u64;

        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            audio_data.extend_from_slice(&chunk);
            downloaded += chunk.len() as u64;

            if content_length > 0 {
                let progress = (downloaded as f32 / content_length as f32) * 100.0;
                debug!(
                    "Download progress: {:.1}% ({}/{} bytes)",
                    progress, downloaded, content_length
                );
            }
        }

        info!(
            "Downloaded {} bytes to memory for track {}",
            audio_data.len(),
            track.id
        );

        // Also cache the file for future use with full API metadata
        if !self.cache.read().await.has_track(&track.id).await? {
            let temp_file = NamedTempFile::new()?;
            tokio::fs::write(temp_file.path(), &audio_data).await?;

            let cache_path = self
                .cache
                .write()
                .await
                .store_track_with_api_metadata(track, temp_file.path(), url)
                .await?;
            info!(
                "Cached track with API metadata at: {}",
                cache_path.display()
            );
        }

        Ok(audio_data)
    }

    async fn download_and_cache_track_url(
        &self,
        track: &Track,
        url: &str,
    ) -> DabResult<Box<dyn ReadSeek>> {
        let response = self.http_client.get(url).send().await?.error_for_status()?;

        let content_length = response.content_length();
        let mut stream = response.bytes_stream();

        // Create a temporary file for streaming
        let temp_file = NamedTempFile::new()?;
        let mut temp_writer = tokio::fs::File::create(temp_file.path()).await?;

        // Stream the data while writing to cache
        let mut downloaded = 0u64;
        use futures_util::StreamExt;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            temp_writer.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;

            if let Some(total) = content_length {
                let progress = (downloaded as f32 / total as f32) * 100.0;
                debug!("Download progress: {:.1}%", progress);
            }
        }

        temp_writer.flush().await?;
        temp_writer.sync_all().await?;
        drop(temp_writer);

        // Move to cache with full API metadata
        let cache_path = self
            .cache
            .write()
            .await
            .store_track_with_api_metadata(track, temp_file.path(), url)
            .await?;
        info!(
            "Track cached with API metadata at: {}",
            cache_path.display()
        );

        // Return file handle for immediate playback
        Ok(Box::new(std::fs::File::open(cache_path)?))
    }

    pub async fn preload_track(&self, track: &Track) -> DabResult<()> {
        if self.cache.read().await.has_track(&track.id).await? {
            debug!("Track already cached: {}", track.id);
            return Ok(());
        }

        info!("Preloading track: {}", track.title);

        // For preloading, we can't get the URL without the PlayerEngine's help
        // This method is now primarily for cache checks
        if track.is_local() {
            // For local tracks, we can preload
            if let Some(local_path) = &track.local_path {
                let _ = self.download_and_cache_track_url(track, local_path).await?;
            }
        } else {
            debug!(
                "Cannot preload online track {} without stream URL",
                track.id
            );
        }

        Ok(())
    }

    /// Preload track with stream URL (for smart preloading)
    pub async fn preload_track_with_url(&self, track: &Track, stream_url: &str) -> DabResult<()> {
        if self.cache.read().await.has_track(&track.id).await? {
            debug!("Track already cached: {}", track.id);
            return Ok(());
        }

        info!("Preloading track with URL: {}", track.title);

        // Start background download (don't wait for completion)
        let track_clone = track.clone();
        let stream_url_clone = stream_url.to_string();
        let download_manager = self.download_manager.clone();

        tokio::spawn(async move {
            if let Err(e) = download_manager
                .start_download(track_clone.clone(), stream_url_clone)
                .await
            {
                warn!(
                    "Failed to start preload for track {}: {}",
                    track_clone.id, e
                );
            }
        });

        Ok(())
    }
}
