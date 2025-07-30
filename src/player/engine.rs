use log::{debug, error, info, warn};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, RwLock};

use super::decoder::AudioDecoder;
use super::loader::AudioLoader;
use super::sink::AudioSink;
use super::{PlayerCommand, PlayerEvent, PlayerState, PlayerStatus, Queue, Track};
use crate::cache::Cache;
use crate::error::{DabError, DabResult};

pub struct PlayerEngine {
    command_tx: mpsc::UnboundedSender<PlayerCommand>,
    event_rx: Arc<RwLock<mpsc::UnboundedReceiver<PlayerEvent>>>,
    _handle: tokio::task::JoinHandle<()>,
}

impl PlayerEngine {
    pub async fn new() -> DabResult<Self> {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let cache = Cache::new().await?;
        let loader = AudioLoader::new(cache);
        let queue = Queue::new();
        let audio_sink = Arc::new(RwLock::new(AudioSink::new()?));

        // Simplified player state - start with basic functionality
        let state = Arc::new(RwLock::new(PlayerState::Stopped));
        let current_track = Arc::new(RwLock::new(None::<Track>));
        let position_ms = Arc::new(RwLock::new(0u32));
        let volume = Arc::new(RwLock::new(0.8f32));

        let handle = {
            let state = state.clone();
            let current_track = current_track.clone();
            let position_ms = position_ms.clone();
            let volume = volume.clone();
            let event_tx = event_tx.clone();
            let audio_sink = audio_sink.clone();

            tokio::spawn(async move {
                Self::run_internal(
                    command_rx,
                    event_tx,
                    state,
                    current_track,
                    position_ms,
                    volume,
                    queue,
                    loader,
                    audio_sink,
                )
                .await;
            })
        };

        Ok(Self {
            command_tx,
            event_rx: Arc::new(RwLock::new(event_rx)),
            _handle: handle,
        })
    }

    async fn run_internal(
        mut command_rx: mpsc::UnboundedReceiver<PlayerCommand>,
        event_tx: mpsc::UnboundedSender<PlayerEvent>,
        state: Arc<RwLock<PlayerState>>,
        current_track: Arc<RwLock<Option<Track>>>,
        position_ms: Arc<RwLock<u32>>,
        volume: Arc<RwLock<f32>>,
        queue: Queue,
        loader: AudioLoader,
        audio_sink: Arc<RwLock<AudioSink>>,
    ) {
        info!("Player engine started");

        while let Some(command) = command_rx.recv().await {
            if let Err(e) = Self::handle_command(
                command,
                &event_tx,
                &state,
                &current_track,
                &position_ms,
                &volume,
                &queue,
                &loader,
                &audio_sink,
            )
            .await
            {
                error!("Error handling command: {}", e);
                let _ = event_tx.send(PlayerEvent::Error(e.to_string()));
            }
        }

        info!("Player engine stopped");
    }

    async fn handle_command(
        command: PlayerCommand,
        event_tx: &mpsc::UnboundedSender<PlayerEvent>,
        state: &Arc<RwLock<PlayerState>>,
        current_track: &Arc<RwLock<Option<Track>>>,
        position_ms: &Arc<RwLock<u32>>,
        volume: &Arc<RwLock<f32>>,
        queue: &Queue,
        loader: &AudioLoader,
        audio_sink: &Arc<RwLock<AudioSink>>,
    ) -> DabResult<()> {
        debug!("Handling command: {:?}", command);

        match command {
            PlayerCommand::LoadAndPlay(track_id) => {
                Self::load_and_play_track(
                    &track_id,
                    event_tx,
                    state,
                    current_track,
                    position_ms,
                    volume,
                    queue,
                    loader,
                    audio_sink,
                )
                .await?;
            }
            PlayerCommand::Play => {
                let current_state = state.read().await.clone();
                match current_state {
                    PlayerState::Paused => {
                        audio_sink.read().await.resume()?;
                        *state.write().await = PlayerState::Playing;
                        let _ = event_tx.send(PlayerEvent::StateChanged(PlayerState::Playing));
                    }
                    PlayerState::Stopped => {
                        if let Some(track) = queue.next_track().await {
                            Self::load_and_play_track(
                                &track.url,
                                event_tx,
                                state,
                                current_track,
                                position_ms,
                                volume,
                                queue,
                                loader,
                                audio_sink,
                            )
                            .await?;
                        }
                    }
                    _ => {}
                }
            }
            PlayerCommand::Pause => {
                if *state.read().await == PlayerState::Playing {
                    audio_sink.read().await.pause()?;
                    *state.write().await = PlayerState::Paused;
                    let _ = event_tx.send(PlayerEvent::StateChanged(PlayerState::Paused));
                }
            }
            PlayerCommand::Resume => {
                if *state.read().await == PlayerState::Paused {
                    audio_sink.read().await.resume()?;
                    *state.write().await = PlayerState::Playing;
                    let _ = event_tx.send(PlayerEvent::StateChanged(PlayerState::Playing));
                }
            }
            PlayerCommand::Stop => {
                audio_sink.read().await.stop()?;
                *state.write().await = PlayerState::Stopped;
                *current_track.write().await = None;
                *position_ms.write().await = 0;
                let _ = event_tx.send(PlayerEvent::StateChanged(PlayerState::Stopped));
            }
            PlayerCommand::Next => {
                if let Some(track) = queue.next_track().await {
                    Self::load_and_play_track(
                        &track.url,
                        event_tx,
                        state,
                        current_track,
                        position_ms,
                        volume,
                        queue,
                        loader,
                        audio_sink,
                    )
                    .await?;
                } else {
                    audio_sink.read().await.stop()?;
                    *state.write().await = PlayerState::Stopped;
                    let _ = event_tx.send(PlayerEvent::StateChanged(PlayerState::Stopped));
                }
            }
            PlayerCommand::Previous => {
                if let Some(track) = queue.previous_track().await {
                    Self::load_and_play_track(
                        &track.url,
                        event_tx,
                        state,
                        current_track,
                        position_ms,
                        volume,
                        queue,
                        loader,
                        audio_sink,
                    )
                    .await?;
                } else {
                    audio_sink.read().await.stop()?;
                    *state.write().await = PlayerState::Stopped;
                    let _ = event_tx.send(PlayerEvent::StateChanged(PlayerState::Stopped));
                }
            }
            PlayerCommand::Seek(pos) => {
                *position_ms.write().await = pos;
                let _ = event_tx.send(PlayerEvent::PositionChanged(pos));
            }
            PlayerCommand::AddToQueue(url) => {
                let track = Track::from_url(&url);
                queue.add_track(track).await;
                let _ = event_tx.send(PlayerEvent::QueueChanged);

                // Start preloading in background
                let loader = loader.clone();
                let track_clone = Track::from_url(&url);
                tokio::spawn(async move {
                    if let Err(e) = loader.preload_track(&track_clone).await {
                        warn!("Failed to preload track: {}", e);
                    }
                });
            }
            PlayerCommand::SetVolume(vol) => {
                let clamped_volume = vol.clamp(0.0, 1.0);
                audio_sink.read().await.set_volume(clamped_volume)?;
                *volume.write().await = clamped_volume;
                let _ = event_tx.send(PlayerEvent::VolumeChanged(clamped_volume));
            }
            PlayerCommand::GetStatus(tx) => {
                let status = PlayerStatus {
                    state: state.read().await.clone(),
                    current_track: current_track.read().await.clone(),
                    position_ms: *position_ms.read().await,
                    duration_ms: 0, // TODO: Get from decoder
                    volume: *volume.read().await,
                    queue_length: queue.len().await,
                };
                let _ = tx.send(status);
            }
        }

        Ok(())
    }

    async fn load_and_play_track(
        track_id: &str,
        event_tx: &mpsc::UnboundedSender<PlayerEvent>,
        state: &Arc<RwLock<PlayerState>>,
        current_track: &Arc<RwLock<Option<Track>>>,
        position_ms: &Arc<RwLock<u32>>,
        volume: &Arc<RwLock<f32>>,
        queue: &Queue,
        loader: &AudioLoader,
        audio_sink: &Arc<RwLock<AudioSink>>,
    ) -> DabResult<()> {
        *state.write().await = PlayerState::Loading;
        let _ = event_tx.send(PlayerEvent::StateChanged(PlayerState::Loading));

        // For now, treat track_id as URL
        let track = Track::from_url(track_id);

        // Add to queue if not already there
        queue.add_track(track.clone()).await;

        info!("Loading track: {}", track.title);

        // Load the audio file
        let audio_source = loader.load_track_seekable(&track).await?;

        // Create decoder
        let decoder = AudioDecoder::from_seekable(audio_source)?;

        // Stop any current playback
        audio_sink.read().await.stop()?;

        // Get current volume from stored volume (not player state)
        let volume_value = *volume.read().await;

        // Start playback
        audio_sink.write().await.play(decoder, volume_value)?;

        // Update current track
        *current_track.write().await = Some(track.clone());
        *position_ms.write().await = 0;

        let _ = event_tx.send(PlayerEvent::TrackChanged(track));

        // Start playing
        *state.write().await = PlayerState::Playing;
        let _ = event_tx.send(PlayerEvent::StateChanged(PlayerState::Playing));

        Ok(())
    }

    pub async fn load_and_play(&mut self, track_id: &str) -> DabResult<()> {
        self.send_command(PlayerCommand::LoadAndPlay(track_id.to_string()))
            .await
    }

    pub async fn play(&mut self) -> DabResult<()> {
        self.send_command(PlayerCommand::Play).await
    }

    pub async fn pause(&mut self) -> DabResult<()> {
        self.send_command(PlayerCommand::Pause).await
    }

    pub async fn resume(&mut self) -> DabResult<()> {
        self.send_command(PlayerCommand::Resume).await
    }

    pub async fn stop(&mut self) -> DabResult<()> {
        self.send_command(PlayerCommand::Stop).await
    }

    pub async fn next(&mut self) -> DabResult<()> {
        self.send_command(PlayerCommand::Next).await
    }

    pub async fn previous(&mut self) -> DabResult<()> {
        self.send_command(PlayerCommand::Previous).await
    }

    pub async fn add_to_queue(&mut self, url: &str) -> DabResult<()> {
        self.send_command(PlayerCommand::AddToQueue(url.to_string()))
            .await
    }

    pub async fn get_status(&mut self) -> DabResult<PlayerStatus> {
        let (tx, rx) = oneshot::channel();
        self.send_command(PlayerCommand::GetStatus(tx)).await?;
        rx.await
            .map_err(|_| DabError::Player("Failed to get status".to_string()))
    }

    async fn send_command(&self, command: PlayerCommand) -> DabResult<()> {
        self.command_tx
            .send(command)
            .map_err(|_| DabError::Player("Failed to send command".to_string()))
    }

    pub async fn next_event(&mut self) -> Option<PlayerEvent> {
        // This is a simplified implementation
        // In a real implementation, we'd have a proper event stream
        None
    }
}
