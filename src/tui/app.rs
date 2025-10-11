use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::{error, info};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use std::io;
use std::sync::Arc;
use tokio::sync::mpsc;

use super::background_tasks::{BackgroundTask, BackgroundTaskProcessor, BackgroundTaskResult};
use super::components::{ListItemType, UnifiedList};
use super::handlers::{KeyHandler, NavigationAction};
use crate::async_client::AsyncNetworkClient;
use crate::cache::Cache;
use crate::error::{DabError, DabResult};
use crate::library::Library;
use crate::player::{PlayerEngine, PlayerEvent, PlayerState, Track};
use crate::search::{DabAlbum, DabArtist, DabTrack};

// Search state management for non-blocking operations
#[derive(Debug, Clone)]
enum SearchState {
    Idle,
    Searching,
    Completed,
    Failed(String),
}

#[derive(Debug, Clone)]
struct SearchResult {
    query: String,
    search_type: SearchType,
    tracks: Vec<Track>,
    raw_tracks: Vec<DabTrack>,
}

// Loading state for UI feedback
#[derive(Debug, Clone)]
enum LoadingState {
    Idle,
    LoadingAlbum(String),
    LoadingArtist(String),
    SearchingArtist(String),
    LoadingDiscography(String),
    BatchLoading(usize, usize), // (current, total)
}

pub struct TuiApp {
    player: PlayerEngine,
    library: Library,
    cache: Arc<tokio::sync::RwLock<Cache>>,
    network_client: AsyncNetworkClient,
    key_handler: KeyHandler,
    should_quit: bool,
    current_view: View,
    view_history: Vec<View>, // Navigation history stack
    // Unified list components
    main_list: UnifiedList,
    queue_list: UnifiedList,
    status_message: Option<String>,
    // Player status display
    current_track: Option<Track>,
    player_state: PlayerState,
    current_position_ms: u32,
    track_duration_ms: u32,
    // Header scrolling state
    header_scroll_offset: usize,
    header_scroll_direction: i8, // -1 for left, 1 for right, 0 for stopped
    header_scroll_delay: u8,     // Counter for scroll timing
    // Search state
    search_mode: bool,
    search_query: String,
    search_results_raw: Vec<DabTrack>, // Store original DabTrack data
    search_type: SearchType,
    // Preserved search state for view switching
    last_search_query: String,
    last_search_type: SearchType,
    search_results_preserved: Vec<Track>, // Preserve search results when switching views
    // Async search state
    search_state: SearchState,
    search_rx: mpsc::UnboundedReceiver<SearchResult>,
    search_tx: mpsc::UnboundedSender<SearchResult>,
    // Extended album/artist detail views
    detailed_album: Option<DabAlbum>,
    detailed_artist: Option<DabArtist>,
    artist_albums: Vec<DabAlbum>,
    // Background task processing
    bg_task_tx: mpsc::UnboundedSender<BackgroundTask>,
    bg_task_rx: mpsc::UnboundedReceiver<BackgroundTaskResult>,
    // Loading state for UI feedback
    loading_state: LoadingState,
}

#[derive(Debug, Clone, PartialEq)]
enum SearchType {
    Track,
    Album,
    Artist,
}

impl SearchType {
    fn as_str(&self) -> &'static str {
        match self {
            SearchType::Track => "track",
            SearchType::Album => "album",
            SearchType::Artist => "artist",
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            SearchType::Track => "Tracks",
            SearchType::Album => "Albums",
            SearchType::Artist => "Artists",
        }
    }

    fn next(&self) -> Self {
        match self {
            SearchType::Track => SearchType::Album,
            SearchType::Album => SearchType::Artist,
            SearchType::Artist => SearchType::Track,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum View {
    Library,
    AlbumDetail,
    Queue,
    Search,
    DetailedAlbum,
    ArtistDiscography,
}

impl TuiApp {
    pub async fn new(
        player: PlayerEngine,
        library: Library,
        network_client: AsyncNetworkClient,
    ) -> DabResult<Self> {
        let cache = Cache::new().await?;
        let key_handler = KeyHandler::new();

        // Create async search channel
        let (search_tx, search_rx) = mpsc::unbounded_channel();

        // Create background task processor
        let (bg_processor, bg_task_tx, bg_task_rx) =
            BackgroundTaskProcessor::new(network_client.clone());

        // Start background task processor
        tokio::spawn(async move {
            bg_processor.run().await;
        });

        Ok(Self {
            player,
            library,
            cache: Arc::new(tokio::sync::RwLock::new(cache)),
            network_client,
            key_handler,
            should_quit: false,
            current_view: View::Library,
            view_history: Vec::new(), // Initialize empty history stack
            // Unified list components
            main_list: UnifiedList::new("Library".to_string()),
            queue_list: UnifiedList::new("Queue".to_string()),
            status_message: None,
            // Player status display
            current_track: None,
            player_state: PlayerState::Stopped,
            current_position_ms: 0,
            track_duration_ms: 0,
            // Header scrolling state
            header_scroll_offset: 0,
            header_scroll_direction: 0,
            header_scroll_delay: 0,
            // Search state
            search_mode: false,
            search_query: String::new(),
            search_results_raw: Vec::new(),
            search_type: SearchType::Track,
            // Preserved search state for view switching
            last_search_query: String::new(),
            last_search_type: SearchType::Track,
            search_results_preserved: Vec::new(),
            // Async search state
            search_state: SearchState::Idle,
            search_rx,
            search_tx,
            detailed_album: None,
            detailed_artist: None,
            artist_albums: Vec::new(),
            // Background task processing
            bg_task_tx,
            bg_task_rx,
            // Loading state
            loading_state: LoadingState::Idle,
        })
    }

    pub async fn run(&mut self) -> DabResult<()> {
        // Setup terminal
        enable_raw_mode().map_err(|e| DabError::Io(e))?;
        let mut stderr = io::stderr();
        execute!(stderr, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stderr);
        let mut terminal = Terminal::new(backend)?;

        info!("TUI started");

        // Initialize lists
        self.refresh_main_list().await;

        // Counter for periodic updates
        let mut update_counter = 0u32;

        loop {
            // Only update player status periodically (every 10 loops = ~500ms)
            update_counter += 1;
            if update_counter % 10 == 0 {
                self.update_player_status().await;
            }

            // Draw UI
            terminal.draw(|f| self.ui(f))?;

            // Handle events
            if event::poll(std::time::Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.handle_key_event(key.code).await?;
                    }
                }
            }

            // Handle player events
            while let Some(event) = self.player.next_event().await {
                self.handle_player_event(event).await;
            }

            // Handle async search results
            while let Ok(search_result) = self.search_rx.try_recv() {
                self.handle_search_result(search_result).await;
            }

            // Handle background task results
            while let Ok(task_result) = self.bg_task_rx.try_recv() {
                self.handle_background_task_result(task_result).await;
            }

            if self.should_quit {
                break;
            }
        }

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        info!("TUI stopped");
        Ok(())
    }

    async fn update_player_status(&mut self) {
        // Update player status for header display
        if let Ok(status) = self.player.get_status().await {
            self.current_track = status.current_track;
            self.player_state = status.state;
            self.current_position_ms = status.position_ms;
            self.track_duration_ms = status.duration_ms;
        }

        // Update header scrolling
        self.update_header_scroll();
    }

    fn update_header_scroll(&mut self) {
        // Only scroll if we have a current track and track info is longer than available space
        if let Some(_) = &self.current_track {
            self.header_scroll_delay = (self.header_scroll_delay + 1) % 30; // Slower scroll speed

            if self.header_scroll_delay == 0 {
                let track_text = self.format_current_track_info();
                let app_title = "Dab Music Player";
                let title_width = app_title.chars().count(); // Use char count for Unicode safety

                // Use a reasonable default width - will be calculated dynamically in render_header
                let available_width: usize = 80; // This will be calculated dynamically in render_header
                let remaining_width = available_width.saturating_sub(title_width + 3); // +3 for " | " separator
                let track_text_chars: Vec<char> = track_text.chars().collect();

                if track_text_chars.len() > remaining_width {
                    match self.header_scroll_direction {
                        0 => {
                            // Start scrolling right after a pause
                            self.header_scroll_direction = 1;
                            self.header_scroll_offset += 1;
                        }
                        1 => {
                            // Scrolling right
                            if self.header_scroll_offset + remaining_width >= track_text_chars.len() {
                                self.header_scroll_direction = -1; // Start scrolling left
                            } else {
                                self.header_scroll_offset += 1;
                            }
                        }
                        -1 => {
                            // Scrolling left
                            if self.header_scroll_offset == 0 {
                                self.header_scroll_direction = 0; // Pause before starting right scroll again
                            } else {
                                self.header_scroll_offset -= 1;
                            }
                        }
                        _ => {}
                    }
                } else {
                    // Track info fits, no scrolling needed
                    self.header_scroll_offset = 0;
                    self.header_scroll_direction = 0;
                }
            }
        } else {
            // No track playing, reset scroll
            self.header_scroll_offset = 0;
            self.header_scroll_direction = 0;
        }
    }

    fn format_current_track_info(&self) -> String {
        if let Some(track) = &self.current_track {
            let position_str = Self::format_duration(self.current_position_ms);
            let duration_str = if self.track_duration_ms > 0 {
                Self::format_duration(self.track_duration_ms)
            } else {
                Self::format_duration(track.duration_ms)
            };

            let state_icon = match self.player_state {
                PlayerState::Playing => "▶",
                PlayerState::Paused => "⏸",
                PlayerState::Stopped => "⏹",
                PlayerState::Loading => "⏳",
                PlayerState::Buffering => "⏳",
            };

            format!(
                "{} {} - {} - {} [{}/{}]",
                state_icon, track.artist, track.album, track.title, position_str, duration_str
            )
        } else {
            // Return empty string when no track is playing, since "Dab Music Player" is shown separately
            String::new()
        }
    }

    fn format_duration(ms: u32) -> String {
        let seconds = ms / 1000;
        let minutes = seconds / 60;
        let seconds = seconds % 60;
        format!("{:02}:{:02}", minutes, seconds)
    }

    fn ui(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Main content
                Constraint::Length(3), // Player controls
                Constraint::Length(1), // Status line
            ])
            .split(f.area());

        // Header
        self.render_header(f, chunks[0]);

        // Main content
        match self.current_view {
            View::Queue => self.queue_list.render(f, chunks[1]),
            View::Search => self.render_search(f, chunks[1]),
            _ => self.main_list.render(f, chunks[1]),
        }

        // Player controls
        // self.render_player_controls(f, chunks[2]);

        // Status line
        self.render_status_line(f, chunks[3]);
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        // Calculate available width for text (minus borders and padding)
        let available_width = (area.width.saturating_sub(4)) as usize; // 2 for borders, 2 for padding

        // Fixed left title
        let app_title = "Dab Music Player";
        let title_width = app_title.chars().count(); // Use char count for Unicode safety

        // Calculate remaining width for track info
        let remaining_width = available_width.saturating_sub(title_width + 3); // +3 for " | " separator

        let display_text = if let Some(_) = &self.current_track {
            let track_info = self.format_current_track_info();
            let track_info_chars: Vec<char> = track_info.chars().collect();

            if track_info_chars.len() <= remaining_width {
                // Track info fits, no scrolling needed
                format!("{} | {}", app_title, track_info)
            } else {
                // Track info is too long, use scrolling for the track part only
                let end_pos = (self.header_scroll_offset + remaining_width).min(track_info_chars.len());
                let scrolled_chars: String = track_info_chars[self.header_scroll_offset..end_pos].iter().collect();
                format!("{} | {}", app_title, scrolled_chars)
            }
        } else {
            // No track playing, just show the app title
            app_title.to_string()
        };

        // Create the paragraph with styled text
        let header_spans = if self.current_track.is_some() {
            // When playing a track, use music-themed styling for track info
            let separator_pos = display_text.find(" | ").unwrap_or(0);
            vec![
                // App title in cyan
                Span::styled(
                    &display_text[..separator_pos.min(display_text.len())],
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                // Separator and track info in green
                Span::styled(
                    &display_text[separator_pos..],
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ]
        } else {
            // Default app title styling
            vec![Span::styled(
                display_text,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )]
        };

        let header = Paragraph::new(Line::from(header_spans))
            .block(Block::default().borders(Borders::ALL))
            .alignment(ratatui::layout::Alignment::Left);

        f.render_widget(header, area);
    }

    fn render_library(&mut self, f: &mut Frame, area: Rect) {
        self.main_list.render(f, area);
    }

    fn render_album_detail(&mut self, f: &mut Frame, area: Rect) {
        // Now handled by unified main_list
        self.main_list.render(f, area);
    }

    fn render_queue(&mut self, f: &mut Frame, area: Rect) {
        // Now handled by unified queue_list
        self.queue_list.render(f, area);
    }

    fn render_search(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Combined search input with type
                Constraint::Min(0),    // Search results
            ])
            .split(area);

        // Combined search input and type on one line
        let combined_text = if self.search_mode {
            format!(
                "Type: {} (Press Tab) | Query: {}_",
                self.search_type.display_name().to_lowercase(),
                self.search_query
            )
        } else if !self.last_search_query.is_empty() {
            format!(
                "Type: {} (Press Tab) | Query: {} (Press '/' to search again)",
                self.last_search_type.display_name().to_lowercase(),
                self.last_search_query
            )
        } else {
            format!(
                "Type: {} (Press Tab) | Press '/' to search",
                self.search_type.display_name().to_lowercase()
            )
        };

        let search_input = Paragraph::new(combined_text)
            .style(if self.search_mode {
                Style::default().fg(Color::Yellow)
            } else if !self.last_search_query.is_empty() {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Gray)
            })
            .block(Block::default().title("Search Music").borders(Borders::ALL));
        f.render_widget(search_input, chunks[0]);

        // Search results using unified list
        if !self.main_list.items.is_empty() {
            self.main_list.render(f, chunks[1]);
        } else if !self.search_mode && self.last_search_query.is_empty() {
            // Only show help when no previous search exists
            let cache_info = if let Ok(cache) = self.cache.try_read() {
                format!(
                    "\nCache: {} tracks ({} MB)\nID3 metadata: tracks with tags",
                    cache.get_track_count(),
                    cache.get_cache_size() / (1024 * 1024)
                )
            } else {
                String::new()
            };

            let help_text = Paragraph::new(format!(
                "Press '/' to start searching\nPress Enter to select a track\nPress Esc to go back{}", 
                cache_info
            ))
            .block(Block::default()
                .title("Search Help")
                .borders(Borders::ALL))
            .style(Style::default().fg(Color::Gray));
            f.render_widget(help_text, chunks[1]);
        }
    }

    fn render_detailed_album(&mut self, f: &mut Frame, area: Rect) {
        if let Some(album) = &self.detailed_album {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(6), // Album Info
                    Constraint::Min(0),    // Tracks
                ])
                .split(area);

            // Album info with more details
            let album_info_text = format!(
                "Title: {}\nArtist: {}\nRelease Date: {}\nGenre: {}\nTracks: {}",
                album.title,
                album.artist,
                album
                    .release_date
                    .as_ref()
                    .unwrap_or(&"Unknown".to_string()),
                album.genre.as_ref().unwrap_or(&"Unknown".to_string()),
                album.track_count.unwrap_or(0)
            );

            let album_info = Paragraph::new(album_info_text).block(
                Block::default()
                    .title("Album Details")
                    .borders(Borders::ALL),
            );
            f.render_widget(album_info, chunks[0]);

            // Tracks using unified list
            self.main_list.render(f, chunks[1]);
        } else {
            self.main_list.render(f, area);
        }
    }

    fn render_artist_discography(&mut self, f: &mut Frame, area: Rect) {
        if let Some(artist) = &self.detailed_artist {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(6), // Artist Info
                    Constraint::Min(0),    // Albums
                ])
                .split(area);

            // Artist info
            let artist_info_text = format!(
                "Artist: {}\nAlbums: {}\nBiography: {}",
                artist.name,
                artist.albums_count.unwrap_or(0),
                artist
                    .biography
                    .as_ref()
                    .map(|b| b.to_string())
                    .unwrap_or_else(|| "No biography available".to_string())
            );

            let artist_info = Paragraph::new(artist_info_text).block(
                Block::default()
                    .title("Artist Information")
                    .borders(Borders::ALL),
            );
            f.render_widget(artist_info, chunks[0]);

            // Albums using unified list
            self.main_list.render(f, chunks[1]);
        } else {
            self.main_list.render(f, area);
        }
    }

    fn render_player_controls(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(60), // Track info
                Constraint::Percentage(40), // Controls
            ])
            .split(area);

        // Track info (placeholder)
        let track_info = Paragraph::new("No track playing")
            .block(Block::default().title("Now Playing").borders(Borders::ALL));
        f.render_widget(track_info, chunks[0]);

        // Controls
        let controls =
            Paragraph::new("Space: Play/Pause | N: Next | P: Previous | /: Search | Q: Quit")
                .block(Block::default().title("Controls").borders(Borders::ALL));
        f.render_widget(controls, chunks[1]);
    }

    fn render_status_line(&self, f: &mut Frame, area: Rect) {
        let status_text = if let Some(ref msg) = self.status_message {
            msg.clone()
        } else {
            // Show loading state if active
            match &self.loading_state {
                LoadingState::LoadingAlbum(name) => format!("⏳ Loading album: {}...", name),
                LoadingState::LoadingArtist(name) => format!("⏳ Loading artist: {}...", name),
                LoadingState::SearchingArtist(name) => format!("🔍 Searching for artist: {}...", name),
                LoadingState::LoadingDiscography(name) => format!("⏳ Loading discography for {}...", name),
                LoadingState::BatchLoading(current, total) => {
                    format!("⏳ Loading albums: {}/{}...", current, total)
                }
                LoadingState::Idle => {
                    format!(
                        "View: {:?} | {} items",
                        self.current_view,
                        self.main_list.items.len()
                    )
                }
            }
        };

        let status_style = match self.loading_state {
            LoadingState::Idle => Style::default().fg(Color::Yellow),
            _ => Style::default().fg(Color::Cyan).add_modifier(Modifier::ITALIC),
        };

        let status = Paragraph::new(status_text).style(status_style);
        f.render_widget(status, area);
    }

    async fn handle_key_event(&mut self, key: KeyCode) -> DabResult<()> {
        // Handle search mode input
        if self.search_mode {
            match key {
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                }
                KeyCode::Tab => {
                    // Switch search type
                    self.search_type = self.search_type.next();
                }
                KeyCode::Enter => {
                    // Perform search
                    if !self.search_query.is_empty() {
                        // Clear preserved results when starting a new search
                        self.search_results_preserved.clear();
                        self.perform_search().await?;
                    }
                    self.search_mode = false;
                }
                KeyCode::Esc => {
                    self.search_mode = false;
                    self.search_query.clear();
                }
                _ => {}
            }
            return Ok(());
        }

        match key {
            KeyCode::Char('q') => self.should_quit = true,

            // Search functionality
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.search_query.clear();
                if self.current_view != View::Search {
                    self.switch_view(View::Search).await;
                }
            }

            // Navigation
            KeyCode::Up | KeyCode::Char('k') => match self.current_view {
                View::Queue => self.queue_list.move_up(),
                _ => self.main_list.move_up(),
            },
            KeyCode::Down | KeyCode::Char('j') => match self.current_view {
                View::Queue => self.queue_list.move_down(),
                _ => self.main_list.move_down(),
            },
            KeyCode::Char('l') => {
                let current_list = match self.current_view {
                    View::Queue => &self.queue_list,
                    _ => &self.main_list,
                };

                if let Ok(Some(action)) = self
                    .key_handler
                    .handle_l_key(current_list, &self.network_client)
                    .await
                {
                    match action {
                        NavigationAction::ShowAlbumDetail(album_id) => {
                            self.show_album_detail_by_id(&album_id).await?;
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Char('h') => {
                // Load artist discography (NON-BLOCKING)
                let current_list = match self.current_view {
                    View::Queue => &self.queue_list,
                    _ => &self.main_list,
                };

                if let Some(item) = current_list.get_selected_item() {
                    if let Some(artist_id) = item.get_artist_id() {
                        // Direct load with artist ID
                        self.loading_state = LoadingState::LoadingDiscography(item.get_artist_name());
                        self.status_message = Some(format!("Loading discography for {}...", item.get_artist_name()));
                        self.bg_task_tx
                            .send(BackgroundTask::LoadDiscography { artist_id })
                            .ok();
                    } else {
                        // Need to search for artist first
                        let artist_name = item.get_artist_name();
                        self.loading_state = LoadingState::SearchingArtist(artist_name.clone());
                        self.status_message = Some(format!("Searching for artist: {}...", artist_name));
                        self.bg_task_tx
                            .send(BackgroundTask::SearchArtist { query: artist_name })
                            .ok();
                    }
                }
            }

            // View switching - use switch_to_main_view to clear history
            KeyCode::Char('1') => self.switch_to_main_view(View::Library).await,
            KeyCode::Char('2') => self.switch_to_main_view(View::Queue).await,
            KeyCode::Char('3') => self.switch_to_main_view(View::Search).await,

            // Player controls
            KeyCode::Char(' ') => self.toggle_playback().await?,
            KeyCode::Char('n') => self.player.next().await?,
            KeyCode::Char('p') => self.player.previous().await?,
            KeyCode::Char('s') => self.player.stop().await?,

            // Track selection - unified Enter key behavior (NON-BLOCKING)
            KeyCode::Enter => {
                let current_list = match self.current_view {
                    View::Queue => &self.queue_list,
                    _ => &self.main_list,
                };

                if let Some(item) = current_list.get_selected_item() {
                    match item {
                        ListItemType::FavoriteAlbum(album) => {
                            // Load album in background for playback
                            self.loading_state = LoadingState::LoadingAlbum(album.title.clone());
                            self.status_message = Some(format!("Loading album: {}...", album.title));
                            self.bg_task_tx
                                .send(BackgroundTask::LoadAlbumForPlay {
                                    album_id: album.id.clone(),
                                })
                                .ok();
                        }
                        ListItemType::Track(track) => {
                            // Play track directly (no network needed)
                            if let Err(e) = self.player.load_and_play_track(track.clone()).await {
                                self.status_message = Some(format!("Failed to play track: {}", e));
                            } else {
                                self.status_message = Some(format!("Playing: {}", track.title));
                            }
                        }
                        ListItemType::DabAlbum(album) => {
                            // Play album tracks directly if available
                            if let Some(tracks) = &album.tracks {
                                let player_tracks: Vec<Track> = tracks
                                    .iter()
                                    .map(|dab_track| Track::from_dab_track(dab_track))
                                    .collect();
                                if !player_tracks.is_empty() {
                                    if let Err(e) = self.player.clear_and_play_tracks(player_tracks.clone()).await {
                                        self.status_message = Some(format!("Failed to play album: {}", e));
                                    } else {
                                        self.status_message = Some(format!("Playing album: {}", album.title));
                                    }
                                }
                            }
                        }
                        ListItemType::QueueTrack { track, .. } => {
                            // Play track directly
                            if let Err(e) = self.player.load_and_play_track(track.clone()).await {
                                self.status_message = Some(format!("Failed to play track: {}", e));
                            } else {
                                self.status_message = Some(format!("Playing: {}", track.title));
                            }
                        }
                        ListItemType::Album(_) => {
                            // Library albums don't have IDs, can't load them
                            self.status_message = Some("Cannot play library album without ID".to_string());
                        }
                    }
                }
            }
            KeyCode::Esc => {
                // Go back to previous view using history stack
                if self.search_mode {
                    // If in search mode, exit search mode first
                    self.search_mode = false;
                    self.search_query.clear();
                } else {
                    // Use navigation history to go back
                    self.go_back().await;
                }
            }
            KeyCode::Char('a') => {
                let current_list = match self.current_view {
                    View::Queue => &self.queue_list,
                    _ => &self.main_list,
                };

                if let Ok(Some(action)) = self
                    .key_handler
                    .handle_a_key(current_list, &self.network_client)
                    .await
                {
                    match action {
                        NavigationAction::AddTrackNext(track) => {
                            self.player.add_track_next(track.clone()).await?;
                            self.status_message =
                                Some(format!("Added {} to queue next", track.title));
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Char('A') => {
                // Clear queue and play all tracks (NON-BLOCKING)
                let current_list = match self.current_view {
                    View::Queue => &self.queue_list,
                    _ => &self.main_list,
                };

                let mut album_ids_to_load = Vec::new();
                let mut immediate_tracks = Vec::new();

                // Collect tracks and albums to load
                for item in &current_list.items {
                    match item {
                        ListItemType::Track(track) => {
                            // Skip album/artist entries in search results
                            if !track.title.starts_with("[Album]") && !track.title.starts_with("[Artist]") {
                                immediate_tracks.push(track.clone());
                            }
                        }
                        ListItemType::FavoriteAlbum(album) => {
                            // Need to load album from network
                            album_ids_to_load.push(album.id.clone());
                        }
                        ListItemType::DabAlbum(album) => {
                            // Can use tracks directly if available
                            if let Some(tracks) = &album.tracks {
                                let player_tracks: Vec<Track> = tracks
                                    .iter()
                                    .map(|dab_track| Track::from_dab_track(dab_track))
                                    .collect();
                                immediate_tracks.extend(player_tracks);
                            }
                        }
                        ListItemType::QueueTrack { track, .. } => {
                            immediate_tracks.push(track.clone());
                        }
                        ListItemType::Album(_) => {
                            // Library albums don't have IDs, skip
                        }
                    }
                }

                if !album_ids_to_load.is_empty() {
                    // Load albums in background
                    self.loading_state = LoadingState::BatchLoading(0, album_ids_to_load.len());
                    self.status_message = Some(format!("Loading {} albums in background...", album_ids_to_load.len()));
                    self.bg_task_tx
                        .send(BackgroundTask::BatchLoadAlbums { album_ids: album_ids_to_load })
                        .ok();
                } else if !immediate_tracks.is_empty() {
                    // Play immediately available tracks
                    if let Err(e) = self.player.clear_and_play_tracks(immediate_tracks.clone()).await {
                        self.status_message = Some(format!("Failed to play tracks: {}", e));
                    } else {
                        self.status_message = Some(format!("Playing {} tracks", immediate_tracks.len()));
                    }
                }
            }

            // Queue management keys - TODO: Implement with unified handlers
            KeyCode::Char('d') => {
                // TODO: Implement queue item removal
                self.status_message = Some("Queue management not yet implemented".to_string());
            }
            KeyCode::Char('c') => {
                // TODO: Implement queue clearing
                self.status_message = Some("Queue management not yet implemented".to_string());
            }

            KeyCode::Char('m') => {
                let current_list = match self.current_view {
                    View::Queue => &self.queue_list,
                    _ => &self.main_list,
                };

                if let Ok(Some(action)) = self
                    .key_handler
                    .handle_m_key(current_list, &self.network_client)
                    .await
                {
                    match action {
                        NavigationAction::AddToFavorites(album) => {
                            match self.library.add_favorite_album(&album) {
                                Ok(is_new) => {
                                    if is_new {
                                        self.status_message = Some(format!(
                                            "Added '{}' by '{}' to library",
                                            album.title, album.artist
                                        ));
                                    } else {
                                        self.status_message = Some(format!(
                                            "'{}' by '{}' is already in library",
                                            album.title, album.artist
                                        ));
                                    }
                                }
                                Err(e) => {
                                    self.status_message =
                                        Some(format!("Failed to add to library: {}", e));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            _ => {}
        }
        Ok(())
    }

    async fn handle_player_event(&mut self, event: PlayerEvent) {
        match event {
            PlayerEvent::StateChanged(state) => {
                self.player_state = state.clone();
                self.status_message = Some(format!("Player state: {:?}", state));
                // Force an immediate status update on state change
                self.update_player_status().await;
            }
            PlayerEvent::TrackChanged(track) => {
                self.current_track = Some(track.clone());
                self.status_message = Some(format!("Now playing: {}", track.title));
                // Reset position and header scroll for new track
                self.current_position_ms = 0;
                self.header_scroll_offset = 0;
                self.header_scroll_direction = 0;
            }
            PlayerEvent::PositionChanged(pos) => {
                // Update position without status message
                self.current_position_ms = pos;
            }
            PlayerEvent::Error(error) => {
                self.status_message = Some(format!("Error: {}", error));
                error!("Player error: {}", error);
            }
            _ => {}
        }
    }

    // Navigation methods removed - now handled by UnifiedList

    async fn switch_view(&mut self, view: View) {
        if self.current_view != view {
            // Push current view to history stack before switching (but not for manual navigation like 1/2/3 keys)
            if !matches!(view, View::Library | View::Queue | View::Search) {
                self.view_history.push(self.current_view.clone());
            }
            
            self.current_view = view.clone();
            self.refresh_main_list().await;

            match view {
                View::Queue => {
                    self.refresh_queue().await;
                }
                _ => {
                    // Lists reset their own selection
                }
            }
        }
    }

    async fn switch_to_main_view(&mut self, view: View) {
        // Clear history when switching to main views (Library, Queue, Search)
        self.view_history.clear();
        
        if self.current_view != view {
            self.current_view = view.clone();
            self.refresh_main_list().await;

            match view {
                View::Queue => {
                    self.refresh_queue().await;
                }
                _ => {
                    // Lists reset their own selection
                }
            }
        }
    }

    async fn go_back(&mut self) {
        if let Some(previous_view) = self.view_history.pop() {
            // Switch back to previous view without adding to history again
            self.current_view = previous_view.clone();
            self.refresh_main_list().await;

            match previous_view {
                View::Queue => {
                    self.refresh_queue().await;
                }
                _ => {
                    // Lists reset their own selection
                }
            }
        }
    }

    async fn refresh_queue(&mut self) {
        let queue = self.player.get_queue();
        let queue_tracks = queue.get_queue().await;
        let current_queue_index = queue.get_current_index().await;

        self.queue_list = UnifiedList::new("Queue".to_string())
            .with_queue_tracks(queue_tracks, current_queue_index)
            .with_help(true);
    }

    async fn refresh_main_list(&mut self) {
        match self.current_view {
            View::Library => {
                let favorite_albums = self.library.get_favorite_albums();
                self.main_list = UnifiedList::new("Favorite Albums".to_string())
                    .with_favorite_albums(favorite_albums)
                    .with_help(true);
            }
            View::AlbumDetail => {
                // AlbumDetail view is now replaced by DetailedAlbum
                // Convert to DetailedAlbum view if needed
                self.main_list = UnifiedList::new("Album Detail".to_string());
            }
            View::Queue => {
                // Queue is handled separately in refresh_queue_list()
            }
            View::Search => {
                // Restore preserved search results or show empty list
                if !self.search_results_preserved.is_empty() {
                    self.main_list = UnifiedList::new(format!(
                        "Search Results: {}",
                        self.last_search_type.display_name()
                    ))
                    .with_tracks(self.search_results_preserved.clone())
                    .with_help(true);
                } else {
                    // Initialize empty search results - show empty list until search is performed
                    self.main_list = UnifiedList::new(format!("Search: {}", self.search_type.display_name()));
                    // Don't add help text for search view initially
                }
            }
            View::DetailedAlbum => {
                if let Some(album) = &self.detailed_album {
                    if let Some(tracks) = &album.tracks {
                        let player_tracks: Vec<Track> = tracks
                            .iter()
                            .map(|dab_track| Track::from_dab_track(dab_track))
                            .collect();
                        self.main_list = UnifiedList::new(format!("Album: {}", album.title))
                            .with_tracks(player_tracks)
                            .with_help(true);
                    } else {
                        self.main_list = UnifiedList::new("Album Detail".to_string());
                    }
                } else {
                    self.main_list = UnifiedList::new("Album Detail".to_string());
                }
            }
            View::ArtistDiscography => {
                if let Some(artist) = &self.detailed_artist {
                    self.main_list = UnifiedList::new(format!("Artist: {}", artist.name))
                        .with_dab_albums(self.artist_albums.clone())
                        .with_help(true);
                } else {
                    self.main_list = UnifiedList::new("Artist Discography".to_string());
                }
            }
        }
    }

    async fn handle_search_result(&mut self, search_result: SearchResult) {
        // Preserve search state for view switching
        self.last_search_query = search_result.query.clone();
        self.last_search_type = search_result.search_type.clone();
        
        // Update main list with search results
        match search_result.search_type {
            SearchType::Track => {
                self.main_list = UnifiedList::new(format!(
                    "Search Results: {}",
                    search_result.search_type.display_name()
                ))
                .with_tracks(search_result.tracks.clone())
                .with_help(true);
                
                // Preserve the results for view switching
                self.search_results_preserved = search_result.tracks;
            }
            SearchType::Album => {
                self.main_list = UnifiedList::new(format!(
                    "Search Results: {}",
                    search_result.search_type.display_name()
                ))
                .with_tracks(search_result.tracks.clone())
                .with_help(true);
                
                // Preserve the results for view switching
                self.search_results_preserved = search_result.tracks;
            }
            SearchType::Artist => {
                // For artist search, check if we have only one unique artist
                let unique_artists: std::collections::HashSet<String> = search_result.tracks
                    .iter()
                    .filter_map(|track| track.artist_id.clone())
                    .collect();

                if unique_artists.len() == 1 {
                    // Only one artist found, auto-navigate to discography
                    let artist_id = unique_artists.into_iter().next().unwrap();
                    info!("Only one artist found in search, auto-navigating to discography: {}", artist_id);
                    
                    match self.network_client.get_artist_discography(artist_id).await {
                        Ok((artist, albums)) => {
                            self.detailed_artist = Some(artist);
                            self.artist_albums = albums;
                            self.switch_view(View::ArtistDiscography).await;
                            self.status_message = Some(format!(
                                "Auto-loaded discography for '{}'",
                                search_result.query
                            ));
                            return;
                        }
                        Err(e) => {
                            self.status_message = Some(format!(
                                "Failed to load discography: {}",
                                e
                            ));
                            // Fall back to showing artist list
                        }
                    }
                }
                
                // Multiple artists or auto-navigation failed, show artist list
                self.main_list = UnifiedList::new(format!(
                    "Search Results: {}",
                    search_result.search_type.display_name()
                ))
                .with_tracks(search_result.tracks.clone())
                .with_help(true);
                
                // Preserve the results for view switching
                self.search_results_preserved = search_result.tracks;
            }
        }

        // Store raw DabTrack data for artist_id extraction
        self.search_results_raw = search_result.raw_tracks;

        // Update search state
        self.search_state = SearchState::Completed;

        // Only show cached count for non-artist auto-navigation cases
        if search_result.search_type != SearchType::Artist || self.current_view == View::Search {
            let cached_count = self.count_cached_tracks().await;
            self.status_message = Some(format!(
                "Found {} {} for '{}' ({} cached)",
                self.main_list.items.len(),
                search_result.search_type.display_name().to_lowercase(),
                search_result.query,
                cached_count
            ));
        }
    }

    async fn perform_search(&mut self) -> DabResult<()> {
        // Update search state
        self.search_state = SearchState::Searching;
        self.status_message = Some(format!(
            "Searching for '{}' in {}...",
            self.search_query,
            self.search_type.display_name().to_lowercase()
        ));

        // Spawn background search task
        let query = self.search_query.clone();
        let search_type = self.search_type.clone();
        let network_client = self.network_client.clone();
        let cache = self.cache.clone();
        let search_tx = self.search_tx.clone();

        tokio::spawn(async move {
            match Self::perform_background_search(
                query.clone(),
                search_type.clone(),
                network_client,
                cache,
            )
            .await
            {
                Ok((tracks, raw_tracks)) => {
                    let search_result = SearchResult {
                        query,
                        search_type,
                        tracks,
                        raw_tracks,
                    };

                    // Send result back to main thread
                    if let Err(e) = search_tx.send(search_result) {
                        log::error!("Failed to send search result: {}", e);
                    }
                }
                Err(e) => {
                    log::error!("Background search failed: {}", e);
                    // TODO: Send error result back to main thread
                }
            }
        });

        Ok(())
    }

    async fn perform_background_search(
        query: String,
        search_type: SearchType,
        network_client: AsyncNetworkClient,
        cache: Arc<tokio::sync::RwLock<Cache>>,
    ) -> DabResult<(Vec<Track>, Vec<DabTrack>)> {
        // Use cache-aware search based on search type
        let cache_read = cache.read().await;

        let (tracks, raw_tracks) = match search_type {
            SearchType::Track => {
                // Get DabTrack results to store artist_id information
                let search_result = network_client
                    .search(query, search_type.as_str().to_string(), 20)
                    .await?;

                let mut tracks = Vec::new();
                let mut raw_tracks = Vec::new();

                for item in search_result.get_results() {
                    match item {
                        crate::search::SearchResultItem::Track(dab_track) => {
                            let track: Track = dab_track.clone().into();

                            // Cache checking now handled by PlayerEngine during playback
                            tracks.push(track);
                            raw_tracks.push(dab_track);
                        }
                        _ => {
                            log::debug!("Skipping non-track item in track search");
                        }
                    }
                }

                (tracks, raw_tracks)
            }
            SearchType::Album => {
                // Get search results and extract albums from tracks
                let search_result = network_client
                    .search(query, search_type.as_str().to_string(), 20)
                    .await?;

                let mut tracks = Vec::new();
                let mut album_keys = std::collections::HashSet::new();

                // Extract tracks first, regardless of the response format
                for item in search_result.get_results() {
                    match item {
                        crate::search::SearchResultItem::Album(album) => {
                            tracks.push(Track {
                                id: album.id.clone(),
                                title: format!("[Album] {}", album.title),
                                artist: album.artist,
                                album: album.title.clone(),
                                duration_ms: album.duration.map(|s| s * 1000).unwrap_or(0),
                                local_path: None,
                                cover_url: album.cover,
                                track_id: None,
                                artist_id: None,
                                album_id: Some(album.id.clone()),
                            });
                        }
                        crate::search::SearchResultItem::Track(track) => {
                            // Extract unique albums from tracks
                            let album_title = track
                                .album_title
                                .as_ref()
                                .map(|s| s.as_str())
                                .unwrap_or("Unknown Album");
                            let album_key = format!("{}:{}", track.artist, album_title);

                            if album_keys.insert(album_key.clone()) {
                                tracks.push(Track {
                                    id: track.album_id.as_ref().unwrap_or(&track.id).clone(),
                                    title: format!("[Album] {}", album_title),
                                    artist: track.artist.clone(),
                                    album: album_title.to_string(),
                                    duration_ms: track.duration.map(|s| s * 1000).unwrap_or(0),
                                    local_path: None,
                                    cover_url: track.album_cover.clone(),
                                    track_id: None,
                                    artist_id: track.artist_id.clone(),
                                    album_id: track.album_id.clone(),
                                });
                            }
                        }
                        _ => {
                            log::debug!("Skipping non-album/track item in album search");
                        }
                    }
                }

                (tracks, Vec::new())
            }
            SearchType::Artist => {
                // Get search results and extract artists from tracks
                let search_result = network_client
                    .search(query, search_type.as_str().to_string(), 20)
                    .await?;

                let mut tracks = Vec::new();
                let mut seen_artist_ids = std::collections::HashSet::new();

                // Update to handle new search result structure with deduplication
                for item in search_result.get_results() {
                    match item {
                        crate::search::SearchResultItem::Artist(artist) => {
                            // Use artist ID for deduplication
                            if seen_artist_ids.insert(artist.id.clone()) {
                                tracks.push(Track {
                                    id: artist.id.clone(),
                                    title: format!("[Artist] {}", artist.name),
                                    artist: artist.name.clone(),
                                    album: format!("{} albums", artist.albums_count.unwrap_or(0)),
                                    duration_ms: 0,
                                    local_path: None,
                                    cover_url: artist.image.as_ref().and_then(|img| img.large.clone()),
                                    track_id: None,
                                    artist_id: Some(artist.id.clone()),
                                    album_id: None,
                                });
                            }
                        }
                        crate::search::SearchResultItem::Track(track) => {
                            // For artist search, create a pseudo-artist from track info
                            // Use artist_id for deduplication if available, otherwise fall back to artist name
                            let dedup_key = track.artist_id.clone().unwrap_or_else(|| format!("track_artist_{}", track.artist));
                            
                            if seen_artist_ids.insert(dedup_key.clone()) {
                                tracks.push(Track {
                                    id: track.artist_id.clone().unwrap_or_else(|| track.id.clone()),
                                    title: format!("[Artist] {}", track.artist),
                                    artist: track.artist.clone(),
                                    album: "From track search".to_string(),
                                    duration_ms: 0,
                                    local_path: None,
                                    cover_url: None,
                                    track_id: None,
                                    artist_id: track.artist_id.clone().or_else(|| Some(dedup_key)),
                                    album_id: None,
                                });
                            }
                        }
                        _ => {
                            log::debug!("Skipping non-artist/track item in artist search");
                        }
                    }
                }

                (tracks, Vec::new())
            }
        };

        drop(cache_read); // Release the lock

        Ok((tracks, raw_tracks))
    }

    async fn count_cached_tracks(&self) -> usize {
        let cache = self.cache.read().await;
        let mut cached_count = 0;
        for item in &self.main_list.items {
            if let Some(track) = item.get_track() {
                if let Ok(true) = cache.has_track(&track.id).await {
                    cached_count += 1;
                }
            }
        }
        cached_count
    }

    async fn toggle_playback(&mut self) -> DabResult<()> {
        // Get current player status to decide what to do
        let status = self.player.get_status().await?;

        match status.state {
            PlayerState::Playing => self.player.pause().await,
            PlayerState::Paused => self.player.resume().await,
            PlayerState::Stopped => {
                if let Some(item) = self.main_list.items.get(0) {
                    if let Some(track) = item.get_track() {
                        self.player.load_and_play_track(track.clone()).await
                    } else {
                        Ok(())
                    }
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }

    async fn play_selected_track(&mut self) -> DabResult<()> {
        let current_list = match self.current_view {
            View::Queue => &self.queue_list,
            _ => &self.main_list,
        };

        if let Some(item) = current_list.get_selected_item() {
            if let Some(track) = item.get_track() {
                self.status_message = Some(format!("Now playing: {}", track.title));
                self.player.load_and_play_track(track.clone()).await?;
            }
        }
        Ok(())
    }

    async fn show_detailed_album(&mut self, track: &Track) -> DabResult<()> {
        if let Some(album_id) = self.extract_album_id(track) {
            self.status_message = Some(format!("Loading album details for {}...", track.album));

            match self.network_client.get_album(album_id).await {
                Ok(album) => {
                    self.detailed_album = Some(album);
                    self.switch_view(View::DetailedAlbum).await;
                    self.status_message = Some("Album details loaded".to_string());
                }
                Err(e) => {
                    self.status_message = Some(format!("Failed to load album: {}", e));
                }
            }
        } else {
            self.status_message = Some("No album ID available for this track".to_string());
        }
        Ok(())
    }

    async fn show_detailed_album_from_dab_album(&mut self, album: &DabAlbum) -> DabResult<()> {
        self.status_message = Some(format!("Loading album details for {}...", album.title));

        match self.network_client.get_album(album.id.clone()).await {
            Ok(detailed_album) => {
                self.detailed_album = Some(detailed_album);
                self.switch_view(View::DetailedAlbum).await;
                self.status_message = Some("Album details loaded".to_string());
            }
            Err(e) => {
                self.status_message = Some(format!("Failed to load album: {}", e));
            }
        }
        Ok(())
    }

    async fn show_artist_discography(&mut self, track: &Track) -> DabResult<()> {
        self.status_message = Some(format!(
            "Searching for artist discography: {}...",
            track.artist
        ));

        // First try to get artist ID from track
        if let Some(artist_id) = self.extract_artist_id(track) {
            info!(
                "Using extracted artist ID: {} for artist '{}'",
                artist_id, track.artist
            );

            match self.network_client.get_artist_discography(artist_id).await {
                Ok((artist, albums)) => {
                    // Check if we got valid artist information
                    if artist.name != "Unknown Artist" && !albums.is_empty() {
                        self.detailed_artist = Some(artist);
                        self.artist_albums = albums;
                        self.switch_view(View::ArtistDiscography).await;
                        self.status_message = Some("Artist discography loaded".to_string());
                        return Ok(());
                    } else {
                        info!("Got 'Unknown Artist' or no albums, trying artist search first");
                    }
                }
                Err(e) => {
                    info!(
                        "Direct discography call failed: {}, trying artist search",
                        e
                    );
                }
            }
        }

        // If direct approach failed, search for the artist first to get proper ID
        info!("Searching for artist '{}' to get proper ID", track.artist);
        match self
            .network_client
            .search(track.artist.clone(), "artist".to_string(), 10)
            .await
        {
            Ok(search_result) => {
                // Find the best matching artist from the flat results array
                for item in search_result.get_results() {
                    if let crate::search::SearchResultItem::Artist(artist) = item {
                        if artist.name.to_lowercase() == track.artist.to_lowercase() {
                            info!(
                                "Found matching artist: {} with ID: {}",
                                artist.name, artist.id
                            );

                            match self
                                .network_client
                                .get_artist_discography(artist.id.clone())
                                .await
                            {
                                Ok((detailed_artist, albums)) => {
                                    let album_count = albums.len();
                                    self.detailed_artist = Some(detailed_artist);
                                    self.artist_albums = albums;
                                    self.switch_view(View::ArtistDiscography).await;
                                    self.status_message = Some(format!(
                                        "Loaded {} albums for {}",
                                        album_count, artist.name
                                    ));
                                    return Ok(());
                                }
                                Err(e) => {
                                    self.status_message = Some(format!(
                                        "Failed to load discography for {}: {}",
                                        artist.name, e
                                    ));
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
                self.status_message = Some(format!(
                    "Artist '{}' not found in search results",
                    track.artist
                ));
            }
            Err(e) => {
                self.status_message = Some(format!("Failed to search for artist: {}", e));
            }
        }

        Ok(())
    }

    fn extract_album_id(&self, track: &Track) -> Option<String> {
        // Try to extract album ID from track information
        if track.title.starts_with("[Album]") {
            // This is an album entry from search results
            Some(track.id.clone())
        } else {
            // First check if we have raw DabTrack data with album_id field
            for dab_track in &self.search_results_raw {
                if dab_track.id == track.id && dab_track.artist == track.artist {
                    if let Some(ref album_id) = dab_track.album_id {
                        info!(
                            "Found album_id in raw DabTrack data: {} for track '{}'",
                            album_id, track.title
                        );
                        return Some(album_id.clone());
                    }
                }
            }

            // For regular tracks, check if we can find the album ID in main list items
            // Look for corresponding album in current list by matching artist and album name
            for item in &self.main_list.items {
                if let Some(search_track) = item.get_track() {
                    if search_track.title.starts_with("[Album]")
                        && search_track.artist == track.artist
                        && search_track.album == track.album
                    {
                        return Some(search_track.id.clone());
                    }
                }
            }

            // If not found in search results, we'll return None instead of using track ID
            info!(
                "No album ID found for track '{}', cannot show album details",
                track.title
            );
            None
        }
    }

    fn extract_artist_id(&self, track: &Track) -> Option<String> {
        // Try to extract artist ID from track information
        if track.title.starts_with("[Artist]") {
            // This is an artist entry from search results
            info!("Using artist ID from artist entry: {}", track.id);
            Some(track.id.clone())
        } else {
            // First check if we have raw DabTrack data with artist_id field
            for dab_track in &self.search_results_raw {
                if dab_track.id == track.id && dab_track.artist == track.artist {
                    if let Some(ref artist_id) = dab_track.artist_id {
                        info!(
                            "Found artist_id in raw DabTrack data: {} for artist '{}'",
                            artist_id, track.artist
                        );
                        return Some(artist_id.clone());
                    }
                }
            }

            // For regular tracks, check if we can find the artist ID in main list items
            // Look for corresponding artist in current list by matching artist name
            for item in &self.main_list.items {
                if let Some(search_track) = item.get_track() {
                    if search_track.title.starts_with("[Artist]")
                        && search_track.artist == track.artist
                    {
                        info!(
                            "Found artist ID in main list: {} for artist '{}'",
                            search_track.id, track.artist
                        );
                        return Some(search_track.id.clone());
                    }
                }
            }

            // If not found in search results, we'll return None so that show_artist_discography
            // can handle it by searching for the artist first
            info!(
                "No artist ID found in search results for '{}', will search for artist",
                track.artist
            );
            None
        }
    }

    pub async fn show_album_detail_by_id(&mut self, album_id: &str) -> DabResult<()> {
        self.status_message = Some(format!("Loading album details..."));

        match self.network_client.get_album(album_id.to_string()).await {
            Ok(album) => {
                self.detailed_album = Some(album);
                self.switch_view(View::DetailedAlbum).await;
                self.status_message = Some("Album details loaded".to_string());
            }
            Err(e) => {
                self.status_message = Some(format!("Failed to load album: {}", e));
            }
        }
        Ok(())
    }

    pub async fn show_artist_discography_by_id(&mut self, artist_id: &str) -> DabResult<()> {
        self.status_message = Some(format!("Loading artist discography..."));

        match self
            .network_client
            .get_artist_discography(artist_id.to_string())
            .await
        {
            Ok((artist, albums)) => {
                self.detailed_artist = Some(artist);
                self.artist_albums = albums;
                self.switch_view(View::ArtistDiscography).await;
                self.status_message = Some("Artist discography loaded".to_string());
            }
            Err(e) => {
                self.status_message = Some(format!("Failed to load artist discography: {}", e));
            }
        }
        Ok(())
    }

    async fn handle_background_task_result(&mut self, result: BackgroundTaskResult) {
        match result {
            BackgroundTaskResult::AlbumLoaded { album, for_play } => {
                // Clear loading state
                self.loading_state = LoadingState::Idle;

                if for_play {
                    // Play the album tracks
                    if let Some(tracks) = &album.tracks {
                        let player_tracks: Vec<Track> = tracks
                            .iter()
                            .map(|dab_track| Track::from_dab_track(dab_track))
                            .collect();
                        if !player_tracks.is_empty() {
                            if let Err(e) = self.player.clear_and_play_tracks(player_tracks.clone()).await {
                                self.status_message = Some(format!("Failed to play album: {}", e));
                            } else {
                                self.status_message = Some(format!("Playing album: {}", album.title));
                            }
                        }
                    }
                } else {
                    // Show album detail
                    self.detailed_album = Some(album.clone());
                    self.switch_view(View::DetailedAlbum).await;
                    self.status_message = Some(format!("Album '{}' loaded", album.title));
                }
            }
            BackgroundTaskResult::AlbumForAddNextLoaded { tracks } => {
                self.loading_state = LoadingState::Idle;

                if !tracks.is_empty() {
                    // Add all tracks next in queue
                    for track in tracks.iter().rev() {
                        if let Err(e) = self.player.add_track_next(track.clone()).await {
                            error!("Failed to add track to queue: {}", e);
                        }
                    }
                    self.status_message = Some(format!("Added {} tracks to queue", tracks.len()));
                }
            }
            BackgroundTaskResult::ArtistSearchCompleted { query, artist_id } => {
                self.loading_state = LoadingState::Idle;

                if let Some(artist_id) = artist_id {
                    // Load discography in background
                    self.loading_state = LoadingState::LoadingDiscography(query.clone());
                    self.bg_task_tx
                        .send(BackgroundTask::LoadDiscography { artist_id })
                        .ok();
                } else {
                    self.status_message = Some(format!("Artist '{}' not found", query));
                }
            }
            BackgroundTaskResult::DiscographyLoaded { artist, albums } => {
                self.loading_state = LoadingState::Idle;

                self.detailed_artist = Some(artist.clone());
                self.artist_albums = albums;
                self.switch_view(View::ArtistDiscography).await;
                self.status_message = Some(format!("Loaded discography for {}", artist.name));
            }
            BackgroundTaskResult::BatchAlbumsLoaded { albums: _, tracks } => {
                self.loading_state = LoadingState::Idle;

                if !tracks.is_empty() {
                    if let Err(e) = self.player.clear_and_play_tracks(tracks.clone()).await {
                        self.status_message = Some(format!("Failed to play tracks: {}", e));
                    } else {
                        self.status_message = Some(format!("Playing {} tracks", tracks.len()));
                    }
                }
            }
            BackgroundTaskResult::Error { task, error } => {
                self.loading_state = LoadingState::Idle;

                let error_msg = match task {
                    BackgroundTask::LoadAlbum { .. } | BackgroundTask::LoadAlbumForPlay { .. } => {
                        format!("Failed to load album: {}", error)
                    }
                    BackgroundTask::SearchArtist { query } => {
                        format!("Failed to search artist '{}': {}", query, error)
                    }
                    BackgroundTask::LoadDiscography { .. } => {
                        format!("Failed to load discography: {}", error)
                    }
                    BackgroundTask::BatchLoadAlbums { .. } => {
                        format!("Failed to load albums: {}", error)
                    }
                    BackgroundTask::LoadAlbumForAddNext { .. } => {
                        format!("Failed to load album for queue: {}", error)
                    }
                };

                self.status_message = Some(error_msg);
                error!("{}", self.status_message.as_ref().unwrap());
            }
        }
    }
}
