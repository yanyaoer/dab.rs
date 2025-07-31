use log::{debug, info};
use reqwest::Client;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;

use super::decoder::ReadSeek;
use super::Track;
use crate::cache::Cache;
use crate::error::{DabError, DabResult};

#[derive(Clone)]
pub struct AudioLoader {
    http_client: Client,
    cache: Arc<tokio::sync::RwLock<Cache>>,
}

impl AudioLoader {
    pub fn new(cache: Cache) -> Self {
        Self {
            http_client: Client::new(),
            cache: Arc::new(tokio::sync::RwLock::new(cache)),
        }
    }

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

        // Move to cache with original URL
        let cache_path = self
            .cache
            .write()
            .await
            .store_track_with_url(&track.id, temp_file.path(), url)
            .await?;
        info!("Track cached at: {}", cache_path.display());

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
}
