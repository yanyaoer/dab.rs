use log::{debug, error, info};
use reqwest::Client;
use std::io::{BufReader, Seek, SeekFrom};
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt};

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
        if track.url.starts_with('/') || track.url.starts_with("file://") {
            let path = track.url.strip_prefix("file://").unwrap_or(&track.url);
            info!("Loading local track: {}", path);
            return Ok(Box::new(std::fs::File::open(path)?));
        }

        // For remote files, we need to download first to enable seeking
        info!("Downloading track for seekable access: {}", track.url);
        self.download_and_cache_track(track).await
    }

    async fn download_and_cache_track(&self, track: &Track) -> DabResult<Box<dyn ReadSeek>> {
        let response = self
            .http_client
            .get(&track.url)
            .send()
            .await?
            .error_for_status()?;

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
            .store_track_with_url(&track.id, temp_file.path(), &track.url)
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
        let _ = self.download_and_cache_track(track).await?;
        Ok(())
    }
}
