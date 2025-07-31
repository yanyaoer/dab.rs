use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::{debug, error, info};
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
use crate::search::{DabAlbum, DabArtist, DabTrack, MusicSearchApi};

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
    search_results_raw: Vec<DabTrack>, // Store original DabTrack data
    search_type: SearchType,
    // Extended album/artist detail views
    detailed_album: Option<DabAlbum>,
    detailed_artist: Option<DabArtist>,
    artist_albums: Vec<DabAlbum>,
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
            search_results_raw: Vec::new(),
            search_type: SearchType::Track,
            detailed_album: None,
            detailed_artist: None,
            artist_albums: Vec::new(),
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
            View::DetailedAlbum => self.render_detailed_album(f, chunks[1]),
            View::ArtistDiscography => self.render_artist_discography(f, chunks[1]),
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
                .enumerate()
                .map(|(i, track)| {
                    let cached_indicator = if !track.url.is_empty() {
                        " [cached]"
                    } else {
                        ""
                    };

                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{}. ", i + 1), Style::default().fg(Color::Gray)),
                        Span::styled(&track.title, Style::default().fg(Color::White)),
                        Span::styled(cached_indicator, Style::default().fg(Color::Green)),
                    ]))
                })
                .collect();

            let list = List::new(items)
                .block(
                    Block::default()
                        .title(format!(
                            "Tracks ({}) - Press 'a' to add next, 'A' to play all",
                            tracks.len()
                        ))
                        .borders(Borders::ALL),
                )
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
                        .title(format!(
                            "Search Results ({}) - Press 'l' for album, 'h' for artist",
                            self.search_results.len()
                        ))
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

            // Tracks
            if let Some(tracks) = &album.tracks {
                let items: Vec<ListItem> = tracks
                    .iter()
                    .enumerate()
                    .map(|(i, track)| {
                        let duration = track
                            .duration
                            .map(|d| format!(" ({}:{:02})", d / 60, d % 60))
                            .unwrap_or_default();
                        ListItem::new(format!("{}. {}{}", i + 1, track.title, duration))
                    })
                    .collect();

                let list = List::new(items)
                    .block(
                        Block::default()
                            .title(format!(
                                "Tracks ({}) - Press 'a' to add next, 'A' to play all, 'h' for artist",
                                tracks.len()
                            ))
                            .borders(Borders::ALL),
                    )
                    .highlight_style(Style::default().bg(Color::DarkGray))
                    .highlight_symbol("► ");

                f.render_stateful_widget(list, chunks[1], &mut self.album_detail_state);
            } else {
                let no_tracks = Paragraph::new("No track details available")
                    .block(Block::default().title("Tracks").borders(Borders::ALL))
                    .style(Style::default().fg(Color::Gray));
                f.render_widget(no_tracks, chunks[1]);
            }
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

            // Albums
            let items: Vec<ListItem> = self
                .artist_albums
                .iter()
                .map(|album| {
                    let release_year = album
                        .release_date
                        .as_ref()
                        .and_then(|date| date.split('-').next())
                        .unwrap_or("Unknown");

                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("{:<40}", album.title),
                            Style::default().fg(Color::White),
                        ),
                        Span::styled(
                            format!("{:<10}", release_year),
                            Style::default().fg(Color::Yellow),
                        ),
                        Span::styled(
                            format!("{} tracks", album.track_count.unwrap_or(0)),
                            Style::default().fg(Color::Gray),
                        ),
                    ]))
                })
                .collect();

            let list = List::new(items)
                .block(Block::default().title("Albums").borders(Borders::ALL))
                .highlight_style(Style::default().bg(Color::DarkGray))
                .highlight_symbol("► ");

            f.render_stateful_widget(list, chunks[1], &mut self.list_state);
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
            KeyCode::Up | KeyCode::Char('k') => self.list_up(),
            KeyCode::Down | KeyCode::Char('j') => self.list_down(),
            KeyCode::Char('l') => {
                match self.current_view {
                    View::Library => {
                        if let Some(selected_index) = self.list_state.selected() {
                            let albums = self.library.get_albums();
                            if let Some(album) = albums.get(selected_index) {
                                self.selected_album = Some(album.clone().clone());
                                self.switch_view(View::AlbumDetail).await;
                            }
                        }
                    }
                    View::Search => {
                        if let Some(selected_index) = self.list_state.selected() {
                            if let Some(track) = self.search_results.get(selected_index).cloned() {
                                self.show_detailed_album(&track).await?;
                            }
                        }
                    }
                    View::ArtistDiscography => {
                        // Show album details from artist discography
                        if let Some(selected_index) = self.list_state.selected() {
                            if let Some(album) = self.artist_albums.get(selected_index).cloned() {
                                self.show_detailed_album_from_dab_album(&album).await?;
                            }
                        }
                    }
                    _ => {}
                }
            }
            KeyCode::Char('h') => {
                match self.current_view {
                    View::Search => {
                        if let Some(selected_index) = self.list_state.selected() {
                            if let Some(track) = self.search_results.get(selected_index).cloned() {
                                self.show_artist_discography(&track).await?;
                            }
                        }
                    }
                    View::DetailedAlbum => {
                        // Go to artist from album view
                        if let Some(album) = &self.detailed_album {
                            let fake_track = Track {
                                id: "temp".to_string(),
                                title: "temp".to_string(),
                                artist: album.artist.clone(),
                                album: album.title.clone(),
                                url: String::new(),
                                duration_ms: 0,
                                local_path: None,
                                cover_url: None,
                            };
                            self.show_artist_discography(&fake_track).await?;
                        }
                    }
                    _ => {}
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
                View::DetailedAlbum => self.play_selected_track().await?,
                View::ArtistDiscography => {
                    // Enter on artist discography shows album details
                    if let Some(selected_index) = self.list_state.selected() {
                        if let Some(album) = self.artist_albums.get(selected_index).cloned() {
                            self.show_detailed_album_from_dab_album(&album).await?;
                        }
                    }
                }
                _ => {}
            },
            KeyCode::Esc => {
                // Go back to previous view
                match self.current_view {
                    View::DetailedAlbum | View::ArtistDiscography => {
                        self.switch_view(View::Search).await;
                    }
                    View::AlbumDetail => {
                        self.switch_view(View::Library).await;
                    }
                    _ => {}
                }
            }
            KeyCode::Char('a') => {
                match self.current_view {
                    View::AlbumDetail => {
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
                    View::DetailedAlbum => {
                        // Add track from detailed album view
                        if let Some(selected_index) = self.album_detail_state.selected() {
                            if let Some(track) = self.tracks.get(selected_index) {
                                // Get stream URL if needed
                                let stream_url = if track.url.is_empty() {
                                    match self.search_api.get_track_stream_url(track, None).await {
                                        Ok(url) => url,
                                        Err(e) => {
                                            self.status_message =
                                                Some(format!("Failed to get stream URL: {}", e));
                                            return Ok(());
                                        }
                                    }
                                } else {
                                    track.url.clone()
                                };

                                self.player.add_next(&stream_url).await?;
                                self.status_message =
                                    Some(format!("Added {} to queue next", track.title));
                            }
                        }
                    }
                    View::Search => {
                        // Add track from search results
                        if let Some(selected_index) = self.list_state.selected() {
                            if let Some(track) = self.search_results.get(selected_index).cloned() {
                                // Get stream URL if needed
                                let stream_url = if track.url.is_empty() {
                                    match self.search_api.get_track_stream_url(&track, None).await {
                                        Ok(url) => url,
                                        Err(e) => {
                                            self.status_message =
                                                Some(format!("Failed to get stream URL: {}", e));
                                            return Ok(());
                                        }
                                    }
                                } else {
                                    track.url.clone()
                                };

                                self.player.add_next(&stream_url).await?;
                                self.status_message =
                                    Some(format!("Added {} to queue next", track.title));
                            }
                        }
                    }
                    _ => {}
                }
            }
            KeyCode::Char('A') => {
                match self.current_view {
                    View::AlbumDetail => {
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
                    View::DetailedAlbum => {
                        // Clear queue and add all tracks from detailed album
                        if let Some(album) = &self.detailed_album {
                            if let Some(tracks) = &album.tracks {
                                self.status_message =
                                    Some("Getting stream URLs for all tracks...".to_string());

                                let mut stream_urls = Vec::new();
                                for dab_track in tracks {
                                    // Get stream URL for each track
                                    match self.search_api.get_stream_url(&dab_track.id, None).await
                                    {
                                        Ok(url) => stream_urls.push(url),
                                        Err(e) => {
                                            self.status_message = Some(format!(
                                                "Failed to get stream URL for {}: {}",
                                                dab_track.title, e
                                            ));
                                            return Ok(());
                                        }
                                    }
                                }

                                self.player.clear_and_play(stream_urls).await?;
                                self.status_message = Some(format!(
                                    "Cleared queue and added all {} tracks from {}",
                                    tracks.len(),
                                    album.title
                                ));
                            }
                        }
                    }
                    View::Search => {
                        // Clear queue and add all search results (only tracks, not albums/artists)
                        let track_results: Vec<_> = self
                            .search_results
                            .iter()
                            .filter(|track| {
                                !track.title.starts_with("[Album]")
                                    && !track.title.starts_with("[Artist]")
                            })
                            .collect();

                        if !track_results.is_empty() {
                            self.status_message =
                                Some("Getting stream URLs for all tracks...".to_string());

                            let mut stream_urls = Vec::new();
                            for track in &track_results {
                                let stream_url = if track.url.is_empty() {
                                    match self.search_api.get_track_stream_url(track, None).await {
                                        Ok(url) => url,
                                        Err(e) => {
                                            self.status_message = Some(format!(
                                                "Failed to get stream URL for {}: {}",
                                                track.title, e
                                            ));
                                            return Ok(());
                                        }
                                    }
                                } else {
                                    track.url.clone()
                                };
                                stream_urls.push(stream_url);
                            }

                            self.player.clear_and_play(stream_urls).await?;
                            self.status_message = Some(format!(
                                "Cleared queue and added {} tracks from search results",
                                track_results.len()
                            ));
                        }
                    }
                    View::ArtistDiscography => {
                        // Clear queue and add all albums from artist (this might be too many tracks)
                        self.status_message = Some("Use 'l' to select an album first, then 'A' to add all tracks from that album".to_string());
                    }
                    _ => {}
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
        match self.current_view {
            View::AlbumDetail | View::DetailedAlbum => {
                let i = match self.album_detail_state.selected() {
                    Some(i) => {
                        if i == 0 {
                            self.tracks.len() - 1
                        } else {
                            i - 1
                        }
                    }
                    None => 0,
                };
                self.album_detail_state.select(Some(i));
            }
            _ => {
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
        }
    }

    fn list_down(&mut self) {
        match self.current_view {
            View::AlbumDetail | View::DetailedAlbum => {
                let i = match self.album_detail_state.selected() {
                    Some(i) => {
                        if i >= self.tracks.len() - 1 {
                            0
                        } else {
                            i + 1
                        }
                    }
                    None => 0,
                };
                self.album_detail_state.select(Some(i));
            }
            _ => {
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
        }
    }

    async fn switch_view(&mut self, view: View) {
        if self.current_view != view {
            self.current_view = view.clone();
            self.refresh_tracks_list().await;

            match view {
                View::AlbumDetail | View::DetailedAlbum => {
                    self.album_detail_state.select(if self.tracks.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
                }
                _ => {
                    self.list_state.select(if self.tracks.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
                }
            }
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
            View::DetailedAlbum => {
                // Convert DabTrack to Track for the detailed album view
                if let Some(album) = &self.detailed_album {
                    if let Some(tracks) = &album.tracks {
                        tracks
                            .iter()
                            .map(|dab_track| Track {
                                id: dab_track.id.clone(),
                                title: dab_track.title.clone(),
                                artist: dab_track.artist.clone(),
                                album: dab_track
                                    .album_title
                                    .as_ref()
                                    .unwrap_or(&album.title)
                                    .clone(),
                                duration_ms: dab_track.duration.map(|s| s * 1000).unwrap_or(0),
                                url: String::new(), // Will be filled when needed for playback
                                local_path: None,
                                cover_url: dab_track.album_cover.clone(),
                            })
                            .collect()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            }
            View::ArtistDiscography => {
                // Convert albums to tracks for display
                self.artist_albums
                    .iter()
                    .map(|album| Track {
                        id: album.id.clone(),
                        title: format!("[Album] {}", album.title),
                        artist: album.artist.clone(),
                        album: album.title.clone(),
                        duration_ms: album.duration.map(|s| s * 1000).unwrap_or(0),
                        url: String::new(),
                        local_path: None,
                        cover_url: album.cover.clone(),
                    })
                    .collect()
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
                // Get DabTrack results to store artist_id information
                let search_result = self
                    .search_api
                    .search(&self.search_query, self.search_type.as_str(), 20)
                    .await?;

                let mut tracks = Vec::new();
                let mut raw_tracks = Vec::new();

                for item in search_result.get_results() {
                    match item {
                        crate::search::SearchResultItem::Track(dab_track) => {
                            let mut track: Track = dab_track.clone().into();

                            // Check cache for existing URL
                            if let Ok(Some(cached_url)) = cache.get_cached_url(&dab_track.id).await
                            {
                                if !cached_url.is_empty() {
                                    track.url = cached_url;
                                }
                            }

                            tracks.push(track);
                            raw_tracks.push(dab_track);
                        }
                        _ => {
                            debug!("Skipping non-track item in track search");
                        }
                    }
                }

                // Store raw DabTrack data for artist_id extraction
                self.search_results_raw = raw_tracks;
                tracks
            }
            SearchType::Album => {
                // Clear raw tracks for album search
                self.search_results_raw.clear();

                // Get search results and extract albums from tracks
                let search_result = self
                    .search_api
                    .search(&self.search_query, self.search_type.as_str(), 20)
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
                                url: String::new(),
                                duration_ms: album.duration.map(|s| s * 1000).unwrap_or(0),
                                local_path: None,
                                cover_url: album.cover,
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
                                    id: track.album_id.unwrap_or_else(|| track.id.clone()),
                                    title: format!("[Album] {}", album_title),
                                    artist: track.artist.clone(),
                                    album: album_title.to_string(),
                                    url: String::new(),
                                    duration_ms: track.duration.map(|s| s * 1000).unwrap_or(0),
                                    local_path: None,
                                    cover_url: track.album_cover.clone(),
                                });
                            }
                        }
                        _ => {
                            debug!("Skipping non-album/track item in album search");
                        }
                    }
                }

                tracks
            }
            SearchType::Artist => {
                // Clear raw tracks for artist search
                self.search_results_raw.clear();

                // Get search results and extract artists from tracks
                let search_result = self
                    .search_api
                    .search(&self.search_query, self.search_type.as_str(), 20)
                    .await?;

                let mut tracks = Vec::new();

                // Update to handle new search result structure
                for item in search_result.get_results() {
                    match item {
                        crate::search::SearchResultItem::Artist(artist) => {
                            tracks.push(Track {
                                id: artist.id.clone(),
                                title: format!("[Artist] {}", artist.name),
                                artist: artist.name.clone(),
                                album: format!("{} albums", artist.albums_count.unwrap_or(0)),
                                url: String::new(),
                                duration_ms: 0,
                                local_path: None,
                                cover_url: artist.image.as_ref().and_then(|img| img.large.clone()),
                            });
                        }
                        crate::search::SearchResultItem::Track(track) => {
                            // For artist search, create a pseudo-artist from track info
                            tracks.push(Track {
                                id: track.id.clone(),
                                title: format!("[Artist] {}", track.artist),
                                artist: track.artist.clone(),
                                album: "From track search".to_string(),
                                url: String::new(),
                                duration_ms: 0,
                                local_path: None,
                                cover_url: None,
                            });
                        }
                        _ => {
                            debug!("Skipping non-artist/track item in artist search");
                        }
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
        let selected_index = match self.current_view {
            View::AlbumDetail | View::DetailedAlbum => self.album_detail_state.selected(),
            _ => self.list_state.selected(),
        };

        if let Some(selected_index) = selected_index {
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
                    
                    // For detailed album view, we need to get stream URL first
                    if self.current_view == View::DetailedAlbum && track.url.is_empty() {
                        match self.search_api.get_track_stream_url(track, None).await {
                            Ok(stream_url) => {
                                self.player.load_and_play(&stream_url).await?;
                            }
                            Err(e) => {
                                self.status_message = Some(format!("Failed to get stream URL: {}", e));
                                return Err(e);
                            }
                        }
                    } else {
                        self.player.load_and_play(&track.url).await?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn show_detailed_album(&mut self, track: &Track) -> DabResult<()> {
        if let Some(album_id) = self.extract_album_id(track) {
            self.status_message = Some(format!("Loading album details for {}...", track.album));

            match self.search_api.get_album_info(&album_id).await {
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

        match self.search_api.get_album_info(&album.id).await {
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

            match self.search_api.get_artist_discography(&artist_id).await {
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
        match self.search_api.search(&track.artist, "artist", 10).await {
            Ok(search_result) => {
                // Find the best matching artist from the flat results array
                for item in search_result.get_results() {
                    if let crate::search::SearchResultItem::Artist(artist) = item {
                        if artist.name.to_lowercase() == track.artist.to_lowercase() {
                            info!(
                                "Found matching artist: {} with ID: {}",
                                artist.name, artist.id
                            );

                            match self.search_api.get_artist_discography(&artist.id).await {
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

            // For regular tracks, check if we can find the album ID in search results
            // Look for corresponding album in search results by matching artist and album name
            for search_track in &self.search_results {
                if search_track.title.starts_with("[Album]")
                    && search_track.artist == track.artist
                    && search_track.album == track.album
                {
                    return Some(search_track.id.clone());
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

            // For regular tracks, check if we can find the artist ID in search results
            // Look for corresponding artist in search results by matching artist name
            for search_track in &self.search_results {
                if search_track.title.starts_with("[Artist]") && search_track.artist == track.artist
                {
                    info!(
                        "Found artist ID in search results: {} for artist '{}'",
                        search_track.id, track.artist
                    );
                    return Some(search_track.id.clone());
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
}
