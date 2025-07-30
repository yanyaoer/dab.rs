use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::{error, info};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use std::io;
use std::sync::Arc;

use crate::cache::Cache;
use crate::error::{DabError, DabResult};
use crate::library::{Album, Library};
use crate::player::{PlayerEngine, PlayerEvent, PlayerState, Track};
use crate::search::MusicSearchApi;

pub struct TuiApp {
    player: PlayerEngine,
    library: Library,
    cache: Arc<tokio::sync::RwLock<Cache>>,
    search_api: MusicSearchApi,
    should_quit: bool,
    current_view: View,
    list_state: ListState,
    album_detail_state: ListState,
    tracks: Vec<Track>,
    selected_album: Option<Album>,
    status_message: Option<String>,
    // Search state
    search_mode: bool,
    search_query: String,
    search_results: Vec<Track>,
    search_type: SearchType,
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
}

impl TuiApp {
    pub async fn new(player: PlayerEngine, library: Library) -> DabResult<Self> {
        let cache = Cache::new().await?;
        Ok(Self {
            player,
            library,
            cache: Arc::new(tokio::sync::RwLock::new(cache)),
            search_api: MusicSearchApi::new(),
            should_quit: false,
            current_view: View::Library,
            list_state: ListState::default(),
            album_detail_state: ListState::default(),
            tracks: Vec::new(),
            selected_album: None,
            status_message: None,
            search_mode: false,
            search_query: String::new(),
            search_results: Vec::new(),
            search_type: SearchType::Track,
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

        // Initialize tracks list
        self.refresh_tracks_list().await;

        loop {
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
            View::Library => self.render_library(f, chunks[1]),
            View::AlbumDetail => self.render_album_detail(f, chunks[1]),
            View::Queue => self.render_queue(f, chunks[1]),
            View::Search => self.render_search(f, chunks[1]),
        }

        // Player controls
        // self.render_player_controls(f, chunks[2]);

        // Status line
        self.render_status_line(f, chunks[3]);
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let title = Paragraph::new("♪ Dab Music Player")
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(title, area);
    }

    fn render_library(&mut self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .library
            .get_albums()
            .iter()
            .map(|album| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:<30}", album.title),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        format!("{:<20}", album.artist),
                        Style::default().fg(Color::Yellow),
                    ),
                ]))
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().title("Library").borders(Borders::ALL))
            .highlight_style(Style::default().bg(Color::DarkGray))
            .highlight_symbol("► ");

        f.render_stateful_widget(list, area, &mut self.list_state);
    }

    fn render_album_detail(&mut self, f: &mut Frame, area: Rect) {
        if let Some(album) = &self.selected_album {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // Album Info
                    Constraint::Min(0),    // Tracks
                ])
                .split(area);

            let album_info = Paragraph::new(format!("Album: {} by {}", album.title, album.artist))
                .block(Block::default().title("Album Info").borders(Borders::ALL));
            f.render_widget(album_info, chunks[0]);

            let tracks = self.library.get_tracks_by_album(&album.title);
            let items: Vec<ListItem> = tracks
                .iter()
                .map(|track| ListItem::new(track.title.clone()))
                .collect();

            let list = List::new(items)
                .block(Block::default().title("Tracks").borders(Borders::ALL))
                .highlight_style(Style::default().bg(Color::DarkGray))
                .highlight_symbol("► ");

            f.render_stateful_widget(list, chunks[1], &mut self.album_detail_state);
        }
    }

    fn render_queue(&self, f: &mut Frame, area: Rect) {
        let queue_content = Paragraph::new("Queue view - Coming soon!")
            .block(Block::default().title("Queue").borders(Borders::ALL));
        f.render_widget(queue_content, area);
    }

    fn render_search(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Search input
                Constraint::Length(3), // Search type selector
                Constraint::Min(0),    // Search results
            ])
            .split(area);

        // Search input box
        let search_text = if self.search_mode {
            format!("Search: {}_", self.search_query)
        } else {
            "Press '/' to search".to_string()
        };

        let search_input = Paragraph::new(search_text)
            .style(if self.search_mode {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::Gray)
            })
            .block(Block::default().title("Search Music").borders(Borders::ALL));
        f.render_widget(search_input, chunks[0]);

        // Search type selector
        let type_text = format!(
            "Type: {} (Press Tab to change: Track → Album → Artist)",
            self.search_type.display_name()
        );
        let type_selector = Paragraph::new(type_text)
            .style(Style::default().fg(Color::Cyan))
            .block(Block::default().title("Search Type").borders(Borders::ALL));
        f.render_widget(type_selector, chunks[1]);

        // Search results
        if !self.search_results.is_empty() {
            let items: Vec<ListItem> = self
                .search_results
                .iter()
                .enumerate()
                .map(|(i, track)| {
                    let cached_indicator = if !track.url.is_empty() {
                        " [cached]"
                    } else {
                        ""
                    };

                    ListItem::new(Line::from(vec![
                        Span::styled(&track.artist, Style::default().fg(Color::Cyan)),
                        Span::raw(" - "),
                        Span::styled(&track.album, Style::default().fg(Color::Blue)),
                        Span::raw(" - "),
                        Span::styled(&track.title, Style::default().fg(Color::White)),
                        Span::styled(cached_indicator, Style::default().fg(Color::Green)),
                    ]))
                })
                .collect();

            let list = List::new(items)
                .block(
                    Block::default()
                        .title(format!("Search Results ({})", self.search_results.len()))
                        .borders(Borders::ALL),
                )
                .highlight_style(Style::default().bg(Color::DarkGray))
                .highlight_symbol("► ");

            f.render_stateful_widget(list, chunks[2], &mut self.list_state);
        } else if self.current_view == View::Search && !self.search_mode {
            let cache_info = if let Ok(cache) = self.cache.try_read() {
                format!(
                    "\nCache: {} tracks ({} MB)\nID3 metadata: tracks with tags",
                    cache.get_track_count(),
                    cache.get_cache_size() / (1024 * 1024)
                )
            } else {
                String::new()
            };

            let help_text = Paragraph::new(format!("Press '/' to start searching\nPress Enter to select a track\nPress Esc to go back{}", cache_info))
                .block(Block::default()
                    .title("Search Help")
                    .borders(Borders::ALL))
                .style(Style::default().fg(Color::Gray));
            f.render_widget(help_text, chunks[1]);
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
            format!(
                "View: {:?} | {} tracks",
                self.current_view,
                self.tracks.len()
            )
        };

        let status = Paragraph::new(status_text).style(Style::default().fg(Color::Yellow));
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
            KeyCode::Up => self.list_up(),
            KeyCode::Down => self.list_down(),
            KeyCode::Char('l') => {
                if let Some(selected_index) = self.list_state.selected() {
                    let albums = self.library.get_albums();
                    if let Some(album) = albums.get(selected_index) {
                        self.selected_album = Some(album.clone().clone());
                        self.switch_view(View::AlbumDetail).await;
                    }
                }
            }

            // View switching
            KeyCode::Char('1') => self.switch_view(View::Library).await,
            KeyCode::Char('2') => self.switch_view(View::Queue).await,
            KeyCode::Char('3') => self.switch_view(View::Search).await,

            // Player controls
            KeyCode::Char(' ') => self.toggle_playback().await?,
            KeyCode::Char('n') => self.player.next().await?,
            KeyCode::Char('p') => self.player.previous().await?,
            KeyCode::Char('s') => self.player.stop().await?,

            // Track selection
            KeyCode::Enter => match self.current_view {
                View::Library => {
                    if let Some(selected_index) = self.list_state.selected() {
                        let albums = self.library.get_albums();
                        if let Some(album) = albums.get(selected_index) {
                            self.selected_album = Some(album.clone().clone());
                            self.switch_view(View::AlbumDetail).await;
                        }
                    }
                }
                View::AlbumDetail => self.play_selected_track().await?,
                View::Search => self.play_selected_track().await?,
                _ => {}
            },
            KeyCode::Char('a') => {
                if self.current_view == View::AlbumDetail {
                    if let (Some(album), Some(selected_index)) = (
                        self.selected_album.as_ref(),
                        self.album_detail_state.selected(),
                    ) {
                        let tracks = self.library.get_tracks_by_album(&album.title);
                        if let Some(track) = tracks.get(selected_index) {
                            self.player.add_next(&track.url).await?;
                            self.status_message =
                                Some(format!("Added {} to queue next", track.title));
                        }
                    }
                }
            }
            KeyCode::Char('A') => {
                if self.current_view == View::AlbumDetail {
                    if let Some(album) = self.selected_album.as_ref() {
                        let tracks = self.library.get_tracks_by_album(&album.title);
                        let urls = tracks.iter().map(|t| t.url.clone()).collect();
                        self.player.clear_and_play(urls).await?;
                        self.status_message = Some(format!(
                            "Cleared queue and added all tracks from {}",
                            album.title
                        ));
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
                self.status_message = Some(format!("Player state: {:?}", state));
            }
            PlayerEvent::TrackChanged(track) => {
                self.status_message = Some(format!("Now playing: {}", track.title));
            }
            PlayerEvent::Error(error) => {
                self.status_message = Some(format!("Error: {}", error));
                error!("Player error: {}", error);
            }
            _ => {}
        }
    }

    fn list_up(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.tracks.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn list_down(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= self.tracks.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    async fn switch_view(&mut self, view: View) {
        if self.current_view != view {
            self.current_view = view;
            self.refresh_tracks_list().await;
            self.list_state.select(if self.tracks.is_empty() {
                None
            } else {
                Some(0)
            });
        }
    }

    async fn refresh_tracks_list(&mut self) {
        self.tracks = match self.current_view {
            View::Library => {
                self.library
                    .get_albums()
                    .into_iter()
                    .map(|album| Track {
                        id: album.title.clone(), // Using album title as ID for simplicity in TUI
                        title: album.title.clone(),
                        artist: album.artist.clone(),
                        album: album.title.clone(),
                        duration_ms: 0,
                        url: String::new(), // Albums don't have a direct URL
                        local_path: None,
                        cover_url: None,
                    })
                    .collect()
            }
            View::AlbumDetail => {
                if let Some(album) = &self.selected_album {
                    self.library.get_tracks_by_album(&album.title)
                } else {
                    Vec::new()
                }
            }
            View::Queue => {
                // TODO: Get tracks from player queue
                Vec::new()
            }
            View::Search => {
                // Return search results
                self.search_results.clone()
            }
        };
    }

    async fn perform_search(&mut self) -> DabResult<()> {
        self.status_message = Some(format!(
            "Searching for '{}' in {}...", 
            self.search_query, 
            self.search_type.display_name().to_lowercase()
        ));

        // Use cache-aware search based on search type
        let cache = self.cache.read().await;
        
        let search_results = match self.search_type {
            SearchType::Track => {
                self.search_api
                    .search_tracks_with_cache(&self.search_query, 20, Some(&*cache))
                    .await?
            }
            SearchType::Album => {
                // Convert albums to tracks for display  
                let albums = self.search_api
                    .search(&self.search_query, self.search_type.as_str(), 20)
                    .await?;
                
                let mut tracks = Vec::new();
                if let crate::search::SearchResults::Albums { albums: dab_albums } = albums.results {
                    for album in dab_albums {
                        // Create a placeholder track for each album
                        tracks.push(Track {
                            id: album.id.clone(),
                            title: format!("[Album] {}", album.title),
                            artist: album.artist,
                            album: album.title.clone(),
                            url: String::new(),
                            duration_ms: album.duration.map(|s| s * 1000).unwrap_or(0),
                            local_path: None,
                            cover_url: album.cover,
                        });
                    }
                }
                tracks
            }
            SearchType::Artist => {
                // Convert artists to tracks for display
                let artists = self.search_api
                    .search(&self.search_query, self.search_type.as_str(), 20)
                    .await?;
                
                let mut tracks = Vec::new();
                if let crate::search::SearchResults::Artists { artists: dab_artists } = artists.results {
                    for artist in dab_artists {
                        // Create a placeholder track for each artist
                        tracks.push(Track {
                            id: artist.id.clone(),
                            title: format!("[Artist] {}", artist.name),
                            artist: artist.name.clone(),
                            album: format!("{} albums", artist.albums_count.unwrap_or(0)),
                            url: String::new(),
                            duration_ms: 0,
                            local_path: None,
                            cover_url: artist.image,
                        });
                    }
                }
                tracks
            }
        };

        drop(cache); // Release the lock
        self.search_results = search_results;

        let cached_count = self.count_cached_tracks().await;
        self.status_message = Some(format!(
            "Found {} {} for '{}' ({} cached)",
            self.search_results.len(),
            self.search_type.display_name().to_lowercase(),
            self.search_query,
            cached_count
        ));

        // Reset list state to top
        self.list_state.select(if self.search_results.is_empty() {
            None
        } else {
            Some(0)
        });

        // Refresh tracks list to show search results
        self.refresh_tracks_list().await;

        Ok(())
    }

    async fn count_cached_tracks(&self) -> usize {
        let cache = self.cache.read().await;
        let mut cached_count = 0;
        for track in &self.search_results {
            if let Ok(true) = cache.has_track(&track.id).await {
                cached_count += 1;
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
                if let Some(track) = self.tracks.get(0) {
                    self.player.load_and_play(&track.url).await
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }

    async fn play_selected_track(&mut self) -> DabResult<()> {
        if let Some(selected_index) = self.list_state.selected() {
            // Get track info first to avoid borrow conflicts
            let (track_id, track_title, needs_url) = {
                if let Some(track) = self.tracks.get(selected_index) {
                    (
                        track.id.clone(),
                        track.title.clone(),
                        self.current_view == View::Search && track.url.is_empty(),
                    )
                } else {
                    return Ok(());
                }
            };

            // If we're in search view and the track doesn't have a URL yet, get it
            if needs_url {
                self.status_message = Some(format!("Getting stream URL for {}...", track_title));

                let cache = self.cache.read().await;
                let track_ref = self.tracks.get(selected_index).unwrap();
                match self
                    .search_api
                    .get_track_stream_url(track_ref, Some(&*cache))
                    .await
                {
                    Ok(stream_url) => {
                        drop(cache);
                        // Update the track in the search results
                        if let Some(search_track) =
                            self.search_results.iter_mut().find(|t| t.id == track_id)
                        {
                            search_track.url = stream_url.clone();
                        }
                        // Update the track in the current tracks list
                        if let Some(current_track) = self.tracks.get_mut(selected_index) {
                            current_track.url = stream_url.clone();
                        }
                        self.status_message = Some(format!("Now playing: {}", track_title));
                        self.player.load_and_play(&stream_url).await?;
                    }
                    Err(e) => {
                        self.status_message = Some(format!("Failed to get stream URL: {}", e));
                        return Err(e);
                    }
                }
            } else {
                // Track already has URL or we're not in search view
                if let Some(track) = self.tracks.get(selected_index) {
                    self.status_message = Some(format!("Now playing: {}", track.title));
                    self.player.load_and_play(&track.url).await?;
                }
            }
        }
        Ok(())
    }
}
