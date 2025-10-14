use crate::async_client::AsyncNetworkClient;
use crate::player::Track;
use crate::search::{DabAlbum, DabArtist};
use log::{debug, error, info};
use tokio::sync::mpsc;

/// Background task types for non-blocking operations
#[derive(Debug, Clone)]
pub enum BackgroundTask {
    LoadAlbum { album_id: String },
    LoadAlbumForPlay { album_id: String },
    SearchArtist { query: String },
    LoadDiscography { artist_id: String },
    BatchLoadAlbums { album_ids: Vec<String> },
    LoadAlbumForAddNext { album_id: String },
}

/// Results from background tasks
#[derive(Debug, Clone)]
pub enum BackgroundTaskResult {
    AlbumLoaded {
        album: DabAlbum,
        for_play: bool,
    },
    AlbumForAddNextLoaded {
        tracks: Vec<Track>,
    },
    ArtistSearchCompleted {
        query: String,
        artist_id: Option<String>,
    },
    DiscographyLoaded {
        artist: DabArtist,
        albums: Vec<DabAlbum>,
    },
    BatchAlbumsLoaded {
        albums: Vec<DabAlbum>,
        tracks: Vec<Track>,
    },
    Error {
        task: BackgroundTask,
        error: String,
    },
}

/// Background task processor
pub struct BackgroundTaskProcessor {
    network_client: AsyncNetworkClient,
    task_rx: mpsc::UnboundedReceiver<BackgroundTask>,
    result_tx: mpsc::UnboundedSender<BackgroundTaskResult>,
}

impl BackgroundTaskProcessor {
    pub fn new(
        network_client: AsyncNetworkClient,
    ) -> (
        Self,
        mpsc::UnboundedSender<BackgroundTask>,
        mpsc::UnboundedReceiver<BackgroundTaskResult>,
    ) {
        let (task_tx, task_rx) = mpsc::unbounded_channel();
        let (result_tx, result_rx) = mpsc::unbounded_channel();

        let processor = Self {
            network_client,
            task_rx,
            result_tx,
        };

        (processor, task_tx, result_rx)
    }

    /// Start processing background tasks
    pub async fn run(mut self) {
        info!("Background task processor started");

        while let Some(task) = self.task_rx.recv().await {
            let network_client = self.network_client.clone();
            let result_tx = self.result_tx.clone();
            let task_clone = task.clone();

            // Process each task in a separate tokio task for concurrency
            tokio::spawn(async move {
                debug!("Processing background task: {:?}", task_clone);

                let result = match task_clone.clone() {
                    BackgroundTask::LoadAlbum { album_id } => {
                        match network_client.get_album(album_id).await {
                            Ok(album) => BackgroundTaskResult::AlbumLoaded {
                                album,
                                for_play: false,
                            },
                            Err(e) => BackgroundTaskResult::Error {
                                task: task_clone,
                                error: e.to_string(),
                            },
                        }
                    }
                    BackgroundTask::LoadAlbumForPlay { album_id } => {
                        match network_client.get_album(album_id).await {
                            Ok(album) => BackgroundTaskResult::AlbumLoaded {
                                album,
                                for_play: true,
                            },
                            Err(e) => BackgroundTaskResult::Error {
                                task: task_clone,
                                error: e.to_string(),
                            },
                        }
                    }
                    BackgroundTask::LoadAlbumForAddNext { album_id } => {
                        match network_client.get_album(album_id).await {
                            Ok(album) => {
                                if let Some(tracks) = &album.tracks {
                                    let player_tracks: Vec<Track> = tracks
                                        .iter()
                                        .map(|dab_track| Track::from_dab_track(dab_track))
                                        .collect();
                                    BackgroundTaskResult::AlbumForAddNextLoaded {
                                        tracks: player_tracks,
                                    }
                                } else {
                                    BackgroundTaskResult::AlbumForAddNextLoaded {
                                        tracks: Vec::new(),
                                    }
                                }
                            }
                            Err(e) => BackgroundTaskResult::Error {
                                task: task_clone,
                                error: e.to_string(),
                            },
                        }
                    }
                    BackgroundTask::SearchArtist { query } => {
                        match network_client
                            .search(query.clone(), "artist".to_string(), 10)
                            .await
                        {
                            Ok(search_result) => {
                                let mut artist_id = None;
                                for item in search_result.get_results() {
                                    if let crate::search::SearchResultItem::Artist(artist) = item {
                                        if artist.name.to_lowercase() == query.to_lowercase() {
                                            artist_id = Some(artist.id.clone());
                                            break;
                                        }
                                    }
                                }
                                BackgroundTaskResult::ArtistSearchCompleted { query, artist_id }
                            }
                            Err(e) => BackgroundTaskResult::Error {
                                task: task_clone,
                                error: e.to_string(),
                            },
                        }
                    }
                    BackgroundTask::LoadDiscography { artist_id } => {
                        match network_client.get_artist_discography(artist_id).await {
                            Ok((artist, albums)) => {
                                BackgroundTaskResult::DiscographyLoaded { artist, albums }
                            }
                            Err(e) => BackgroundTaskResult::Error {
                                task: task_clone,
                                error: e.to_string(),
                            },
                        }
                    }
                    BackgroundTask::BatchLoadAlbums { album_ids } => {
                        let mut albums = Vec::new();
                        let mut all_tracks = Vec::new();

                        for album_id in album_ids {
                            match network_client.get_album(album_id).await {
                                Ok(album) => {
                                    if let Some(tracks) = &album.tracks {
                                        let player_tracks: Vec<Track> = tracks
                                            .iter()
                                            .map(|dab_track| Track::from_dab_track(dab_track))
                                            .collect();
                                        all_tracks.extend(player_tracks);
                                    }
                                    albums.push(album);
                                }
                                Err(e) => {
                                    error!("Failed to load album in batch: {}", e);
                                    // Continue with other albums even if one fails
                                }
                            }
                        }

                        BackgroundTaskResult::BatchAlbumsLoaded {
                            albums,
                            tracks: all_tracks,
                        }
                    }
                };

                if let Err(e) = result_tx.send(result) {
                    error!("Failed to send background task result: {}", e);
                }
            });
        }

        info!("Background task processor stopped");
    }
}
