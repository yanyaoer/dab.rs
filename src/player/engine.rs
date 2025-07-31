use log::{debug, error, info, warn};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, RwLock};

use super::decoder::AudioDecoder;
use super::loader::AudioLoader;
use super::sink::AudioSink;
use super::queue::StreamUrl;
use super::{PlayerCommand, PlayerEvent, PlayerState, PlayerStatus, Queue, Track, RepeatMode};
use crate::cache::Cache;
use crate::error::{DabError, DabResult};
use crate::search::MusicSearchApi;

pub struct PlayerEngine {
    command_tx: mpsc::UnboundedSender<PlayerCommand>,
    event_rx: Arc<RwLock<mpsc::UnboundedReceiver<PlayerEvent>>>,
    queue: Arc<Queue>,
    search_api: Arc<MusicSearchApi>,
    stream_url_cache: Arc<RwLock<std::collections::HashMap<String, StreamUrl>>>,
    repeat_mode: Arc<RwLock<RepeatMode>>,
    _handle: tokio::task::JoinHandle<()>,
}

impl PlayerEngine {
    pub async fn new() -> DabResult<Self> {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let cache = Cache::new().await?;
        let loader = AudioLoader::new(cache);
        let queue = Arc::new(Queue::new());
        let audio_sink = Arc::new(RwLock::new(AudioSink::new()?));

        // Simplified player state - start with basic functionality
        let state = Arc::new(RwLock::new(PlayerState::Stopped));
        let current_track = Arc::new(RwLock::new(None::<Track>));
        let position_ms = Arc::new(RwLock::new(0u32));
        let volume = Arc::new(RwLock::new(0.8f32));
        let repeat_mode = Arc::new(RwLock::new(RepeatMode::Off));

        let handle = {
            let state = state.clone();
            let current_track = current_track.clone();
            let position_ms = position_ms.clone();
            let volume = volume.clone();
            let repeat_mode = repeat_mode.clone();
            let event_tx = event_tx.clone();
            let audio_sink = audio_sink.clone();
            let queue = queue.clone();

            tokio::spawn(async move {
                Self::run_internal(
                    command_rx,
                    event_tx,
                    state,
                    current_track,
                    position_ms,
                    volume,
                    repeat_mode,
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
            queue,
            search_api: Arc::new(MusicSearchApi::new()),
            stream_url_cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
            repeat_mode,
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
        repeat_mode: Arc<RwLock<RepeatMode>>,
        queue: Arc<Queue>,
        loader: AudioLoader,
        audio_sink: Arc<RwLock<AudioSink>>,
    ) {
        let search_api = Arc::new(MusicSearchApi::new());
        let stream_url_cache = Arc::new(RwLock::new(std::collections::HashMap::<String, StreamUrl>::new()));
        
        info!("Player engine started");

        // Start auto-advance monitoring task
        let _auto_advance_task = {
            let state = state.clone();
            let current_track = current_track.clone();
            let repeat_mode = repeat_mode.clone();
            let queue = queue.clone();
            let event_tx = event_tx.clone();
            let audio_sink = audio_sink.clone();
            let loader = loader.clone();
            let search_api = search_api.clone();
            let stream_url_cache = stream_url_cache.clone();
            let position_ms = position_ms.clone();
            let volume = volume.clone();
            
            tokio::spawn(async move {
                Self::auto_advance_monitor(
                    state,
                    current_track,
                    repeat_mode,
                    queue,
                    event_tx,
                    audio_sink,
                    loader,
                    search_api,
                    stream_url_cache,
                    position_ms,
                    volume,
                ).await;
            })
        };

        while let Some(command) = command_rx.recv().await {
            if let Err(e) = Self::handle_command(
                command,
                &event_tx,
                &state,
                &current_track,
                &position_ms,
                &volume,
                &repeat_mode,
                &queue,
                &loader,
                &audio_sink,
                &search_api,
                &stream_url_cache,
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
        repeat_mode: &Arc<RwLock<RepeatMode>>,
        queue: &Arc<Queue>,
        loader: &AudioLoader,
        audio_sink: &Arc<RwLock<AudioSink>>,
        search_api: &Arc<MusicSearchApi>,
        stream_url_cache: &Arc<RwLock<std::collections::HashMap<String, StreamUrl>>>,
    ) -> DabResult<()> {
        debug!("Handling command: {:?}", command);

        match command {
            PlayerCommand::LoadAndPlay(track_identifier) => {
                // Try to find the track in the queue first
                let queue_tracks = queue.get_queue().await;
                let track = if let Some(found_track) = queue_tracks.iter().find(|t| t.id == track_identifier || t.local_path.as_ref() == Some(&track_identifier)) {
                    found_track.clone()
                } else {
                    // Fallback: create track from URL for backwards compatibility
                    Track::from_url(&track_identifier)
                };
                
                Self::load_and_play_track_internal(
                    &track,
                    event_tx,
                    state,
                    current_track,
                    position_ms,
                    volume,
                    repeat_mode,
                    queue,
                    loader,
                    audio_sink,
                    search_api,
                    stream_url_cache,
                )
                .await?;
            }
            PlayerCommand::LoadAndPlayTrack(track) => {
                Self::load_and_play_track_internal(
                    &track,
                    event_tx,
                    state,
                    current_track,
                    position_ms,
                    volume,
                    repeat_mode,
                    queue,
                    loader,
                    audio_sink,
                    search_api,
                    stream_url_cache,
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
                            Self::load_and_play_track_internal(
                                &track,
                                event_tx,
                                state,
                                current_track,
                                position_ms,
                                volume,
                                repeat_mode,
                                queue,
                                loader,
                                audio_sink,
                                search_api,
                                stream_url_cache,
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
                    Self::load_and_play_track_internal(
                        &track,
                        event_tx,
                        state,
                        current_track,
                        position_ms,
                        volume,
                        repeat_mode,
                        queue,
                        loader,
                        audio_sink,
                        search_api,
                        stream_url_cache,
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
                    Self::load_and_play_track_internal(
                        &track,
                        event_tx,
                        state,
                        current_track,
                        position_ms,
                        volume,
                        repeat_mode,
                        queue,
                        loader,
                        audio_sink,
                        search_api,
                        stream_url_cache,
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
            PlayerCommand::AddToQueue(track_identifier) => {
                // Try to resolve track from identifier - could be URL, track ID, or local path
                let track = Self::resolve_track_from_identifier(&track_identifier, search_api).await
                    .unwrap_or_else(|_| Track::from_url(&track_identifier));
                
                queue.add_track(track.clone()).await;
                let _ = event_tx.send(PlayerEvent::QueueChanged);

                // Start preloading in background
                let loader = loader.clone();
                tokio::spawn(async move {
                    if let Err(e) = loader.preload_track(&track).await {
                        warn!("Failed to preload track: {}", e);
                    }
                });
            }
            PlayerCommand::AddTrackToQueue(track) => {
                queue.add_track(track.clone()).await;
                let _ = event_tx.send(PlayerEvent::QueueChanged);

                // Start preloading in background
                let loader = loader.clone();
                tokio::spawn(async move {
                    if let Err(e) = loader.preload_track(&track).await {
                        warn!("Failed to preload track: {}", e);
                    }
                });
            }
            PlayerCommand::AddNext(track_identifier) => {
                // Try to resolve track from identifier
                let track = Self::resolve_track_from_identifier(&track_identifier, search_api).await
                    .unwrap_or_else(|_| Track::from_url(&track_identifier));
                
                queue.add_track_next(track).await;
                let _ = event_tx.send(PlayerEvent::QueueChanged);
            }
            PlayerCommand::AddTrackNext(track) => {
                queue.add_track_next(track).await;
                let _ = event_tx.send(PlayerEvent::QueueChanged);
            }
            PlayerCommand::ClearAndPlay(track_identifiers) => {
                queue.clear().await;
                for identifier in track_identifiers {
                    // Try to resolve each track from identifier
                    let track = Self::resolve_track_from_identifier(&identifier, search_api).await
                        .unwrap_or_else(|_| Track::from_url(&identifier));
                    queue.add_track(track).await;
                }
                if let Some(track) = queue.next_track().await {
                    Self::load_and_play_track_internal(
                        &track,
                        event_tx,
                        state,
                        current_track,
                        position_ms,
                        volume,
                        repeat_mode,
                        queue,
                        loader,
                        audio_sink,
                        search_api,
                        stream_url_cache,
                    )
                    .await?;
                }
                let _ = event_tx.send(PlayerEvent::QueueChanged);
            }
            PlayerCommand::ClearAndPlayTracks(tracks) => {
                queue.clear().await;
                for track in tracks {
                    queue.add_track(track).await;
                }
                if let Some(track) = queue.next_track().await {
                    Self::load_and_play_track_internal(
                        &track,
                        event_tx,
                        state,
                        current_track,
                        position_ms,
                        volume,
                        repeat_mode,
                        queue,
                        loader,
                        audio_sink,
                        search_api,
                        stream_url_cache,
                    )
                    .await?;
                }
                let _ = event_tx.send(PlayerEvent::QueueChanged);
            }
            PlayerCommand::SetVolume(vol) => {
                let clamped_volume = vol.clamp(0.0, 1.0);
                audio_sink.read().await.set_volume(clamped_volume)?;
                *volume.write().await = clamped_volume;
                let _ = event_tx.send(PlayerEvent::VolumeChanged(clamped_volume));
            }
            PlayerCommand::SetRepeatMode(mode) => {
                *repeat_mode.write().await = mode;
                if let Err(e) = queue.send_command(super::queue::QueueCommand::SetRepeat(mode)).await {
                    warn!("Failed to set repeat mode in queue: {}", e);
                }
                let _ = event_tx.send(PlayerEvent::RepeatModeChanged(mode));
            }
            PlayerCommand::GetStatus(tx) => {
                let status = PlayerStatus {
                    state: state.read().await.clone(),
                    current_track: current_track.read().await.clone(),
                    position_ms: *position_ms.read().await,
                    duration_ms: 0, // TODO: Get from decoder
                    volume: *volume.read().await,
                    queue_length: queue.len().await,
                    repeat_mode: *repeat_mode.read().await,
                };
                let _ = tx.send(status);
            }
        }

        Ok(())
    }

    async fn load_and_play_track_internal(
        track: &Track,
        event_tx: &mpsc::UnboundedSender<PlayerEvent>,
        state: &Arc<RwLock<PlayerState>>,
        current_track: &Arc<RwLock<Option<Track>>>,
        position_ms: &Arc<RwLock<u32>>,
        volume: &Arc<RwLock<f32>>,
        _repeat_mode: &Arc<RwLock<RepeatMode>>,
        _queue: &Arc<Queue>,
        loader: &AudioLoader,
        audio_sink: &Arc<RwLock<AudioSink>>,
        search_api: &Arc<MusicSearchApi>,
        stream_url_cache: &Arc<RwLock<std::collections::HashMap<String, StreamUrl>>>,
    ) -> DabResult<()> {
        *state.write().await = PlayerState::Loading;
        let _ = event_tx.send(PlayerEvent::StateChanged(PlayerState::Loading));

        info!("Loading track: {}", track.title);

        // Get stream URL for the track
        let stream_url = Self::get_stream_url(track, search_api, stream_url_cache).await?;

        // Create a track with the stream URL for loading
        let mut track_with_url = track.clone();
        // Set the local_path to the stream URL so the loader can access it
        track_with_url.local_path = Some(stream_url.clone());

        // Load the audio file using the track with stream URL
        let audio_source = loader.load_track_seekable(&track_with_url).await?;

        // Create decoder
        let decoder = AudioDecoder::from_seekable(audio_source)?;

        // Stop any current playback
        audio_sink.read().await.stop()?;

        // Get current volume from stored volume (not player state)
        let volume_value = *volume.read().await;

        // Start playback
        audio_sink.write().await.play(decoder, volume_value)?;

        // Update current track with original track metadata (not the URL-based track)
        *current_track.write().await = Some(track.clone());
        *position_ms.write().await = 0;

        let _ = event_tx.send(PlayerEvent::TrackChanged(track.clone()));

        // Start playing
        *state.write().await = PlayerState::Playing;
        let _ = event_tx.send(PlayerEvent::StateChanged(PlayerState::Playing));

        Ok(())
    }

    pub async fn load_and_play(&mut self, track_or_url: &str) -> DabResult<()> {
        // For backwards compatibility, treat input as URL
        self.send_command(PlayerCommand::LoadAndPlay(track_or_url.to_string()))
            .await
    }

    pub async fn load_and_play_track(&mut self, track: Track) -> DabResult<()> {
        self.send_command(PlayerCommand::LoadAndPlayTrack(track)).await
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

    pub async fn add_track_to_queue(&mut self, track: Track) -> DabResult<()> {
        self.send_command(PlayerCommand::AddTrackToQueue(track)).await
    }

    pub async fn add_next(&mut self, url: &str) -> DabResult<()> {
        self.send_command(PlayerCommand::AddNext(url.to_string()))
            .await
    }

    pub async fn add_track_next(&mut self, track: Track) -> DabResult<()> {
        self.send_command(PlayerCommand::AddTrackNext(track)).await
    }

    pub async fn clear_and_play(&mut self, urls: Vec<String>) -> DabResult<()> {
        self.send_command(PlayerCommand::ClearAndPlay(urls)).await
    }

    pub async fn clear_and_play_tracks(&mut self, tracks: Vec<Track>) -> DabResult<()> {
        self.send_command(PlayerCommand::ClearAndPlayTracks(tracks)).await
    }

    pub async fn get_status(&mut self) -> DabResult<PlayerStatus> {
        let (tx, rx) = oneshot::channel();
        self.send_command(PlayerCommand::GetStatus(tx)).await?;
        rx.await
            .map_err(|_| DabError::Player("Failed to get status".to_string()))
    }

    pub async fn set_repeat_mode(&mut self, mode: RepeatMode) -> DabResult<()> {
        self.send_command(PlayerCommand::SetRepeatMode(mode)).await
    }

    pub async fn get_repeat_mode(&self) -> RepeatMode {
        *self.repeat_mode.read().await
    }

    async fn send_command(&self, command: PlayerCommand) -> DabResult<()> {
        self.command_tx
            .send(command)
            .map_err(|_| DabError::Player("Failed to send command".to_string()))
    }

    pub async fn next_event(&mut self) -> Option<PlayerEvent> {
        // This is a simplified implementation
        // In a real implementation, we'd have a proper event stream
        if let Ok(mut event_rx) = self.event_rx.try_write() {
            if let Ok(event) = event_rx.try_recv() {
                return Some(event);
            }
        }
        None
    }
    
    pub fn get_queue(&self) -> Arc<Queue> {
        self.queue.clone()
    }

    /// Get or fetch stream URL for a track
    async fn get_stream_url(
        track: &Track,
        search_api: &Arc<MusicSearchApi>,
        stream_url_cache: &Arc<RwLock<std::collections::HashMap<String, StreamUrl>>>,
    ) -> DabResult<String> {
        // If it's a local track, return the local path
        if track.is_local() {
            return Ok(track.local_path.as_ref().unwrap().clone());
        }

        // Check if we have a cached stream URL that's not expired
        {
            let cache = stream_url_cache.read().await;
            if let Some(stream_url) = cache.get(&track.id) {
                if !stream_url.is_expired() {
                    return Ok(stream_url.url.clone());
                }
            }
        }

        // If track doesn't require stream URL fetching, return error
        if !track.requires_stream_url() {
            return Err(DabError::Player(format!(
                "Track {} requires stream URL but no track_id available",
                track.title
            )));
        }

        // Fetch new stream URL from API
        let track_id = track.track_id.as_ref().unwrap();
        let stream_url_string = search_api.get_stream_url(track_id, None).await?;

        // Cache the new URL with default expiration (1 hour)
        let stream_url = StreamUrl::new(stream_url_string.clone(), None);
        {
            let mut cache = stream_url_cache.write().await;
            cache.insert(track.id.clone(), stream_url);
        }

        Ok(stream_url_string)
    }

    /// Resolve a track from an identifier (could be track ID, URL, or local path)
    async fn resolve_track_from_identifier(
        identifier: &str,
        search_api: &Arc<MusicSearchApi>,
    ) -> DabResult<Track> {
        // If it looks like a local path or URL, create track from URL
        if identifier.starts_with("/") || identifier.starts_with("file://") || identifier.starts_with("http") {
            return Ok(Track::from_url(identifier));
        }

        // Otherwise, try to search for the track by ID
        // First try searching for track by title/artist if the identifier looks like metadata
        if identifier.contains(" - ") {
            let parts: Vec<&str> = identifier.splitn(2, " - ").collect();
            if parts.len() == 2 {
                let query = format!("{} {}", parts[0], parts[1]);
                if let Ok(search_result) = search_api.search(&query, "track", 1).await {
                    if let Some(first_result) = search_result.get_results().first() {
                        if let crate::search::SearchResultItem::Track(dab_track) = first_result {
                            return Ok(Track::from_dab_track(dab_track));
                        }
                    }
                }
            }
        }

        // If all else fails, treat as URL
        Ok(Track::from_url(identifier))
    }

    /// Auto-advance monitor that checks for track completion and advances queue
    async fn auto_advance_monitor(
        state: Arc<RwLock<PlayerState>>,
        current_track: Arc<RwLock<Option<Track>>>,
        repeat_mode: Arc<RwLock<RepeatMode>>,
        queue: Arc<Queue>,
        event_tx: mpsc::UnboundedSender<PlayerEvent>,
        audio_sink: Arc<RwLock<AudioSink>>,
        loader: AudioLoader,
        search_api: Arc<MusicSearchApi>,
        stream_url_cache: Arc<RwLock<std::collections::HashMap<String, StreamUrl>>>,
        position_ms: Arc<RwLock<u32>>,
        volume: Arc<RwLock<f32>>,
    ) {
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        
        info!("Auto-advance monitor started");
        
        loop {
            interval.tick().await;
            
            // Check if we're currently playing and the sink is empty (track ended)
            let current_state = state.read().await.clone();
            if current_state == PlayerState::Playing {
                let is_empty = audio_sink.read().await.is_empty();
                
                if is_empty {
                    // Track has ended, handle auto-advance
                    info!("Track ended, handling auto-advance");
                    let _ = event_tx.send(PlayerEvent::TrackEnded);
                    
                    let current_repeat_mode = *repeat_mode.read().await;
                    
                    match current_repeat_mode {
                        RepeatMode::One => {
                            // Repeat current track
                            if let Some(track) = current_track.read().await.clone() {
                                info!("Repeating current track: {}", track.title);
                                if let Err(e) = Self::load_and_play_track_internal(
                                    &track,
                                    &event_tx,
                                    &state,
                                    &current_track,
                                    &position_ms,
                                    &volume,
                                    &repeat_mode,
                                    &queue,
                                    &loader,
                                    &audio_sink,
                                    &search_api,
                                    &stream_url_cache,
                                ).await {
                                    error!("Failed to repeat track: {}", e);
                                    let _ = event_tx.send(PlayerEvent::Error(e.to_string()));
                                }
                            }
                        }
                        RepeatMode::All | RepeatMode::Off => {
                            // Try to advance to next track
                            if let Some(next_track) = queue.next_track().await {
                                info!("Auto-advancing to next track: {}", next_track.title);
                                if let Err(e) = Self::load_and_play_track_internal(
                                    &next_track,
                                    &event_tx,
                                    &state,
                                    &current_track,
                                    &position_ms,
                                    &volume,
                                    &repeat_mode,
                                    &queue,
                                    &loader,
                                    &audio_sink,
                                    &search_api,
                                    &stream_url_cache,
                                ).await {
                                    error!("Failed to advance to next track: {}", e);
                                    let _ = event_tx.send(PlayerEvent::Error(e.to_string()));
                                }
                            } else {
                                // No more tracks, stop playback
                                info!("No more tracks in queue, stopping playback");
                                *state.write().await = PlayerState::Stopped;
                                *current_track.write().await = None;
                                *position_ms.write().await = 0;
                                let _ = event_tx.send(PlayerEvent::StateChanged(PlayerState::Stopped));
                            }
                        }
                    }
                }
            }
        }
    }
}
