use std::io;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use log::{info, error};

use crate::error::{DabResult, DabError};
use crate::player::{PlayerEngine, PlayerEvent, PlayerState, Track};
use crate::library::Library;

pub struct TuiApp {
    player: PlayerEngine,
    library: Library,
    should_quit: bool,
    current_view: View,
    list_state: ListState,
    tracks: Vec<Track>,
    status_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum View {
    Library,
    Queue,
    Search,
}

impl TuiApp {
    pub async fn new(player: PlayerEngine, library: Library) -> DabResult<Self> {
        Ok(Self {
            player,
            library,
            should_quit: false,
            current_view: View::Library,
            list_state: ListState::default(),
            tracks: Vec::new(),
            status_message: None,
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
                Constraint::Length(3),  // Header
                Constraint::Min(0),     // Main content
                Constraint::Length(3),  // Player controls
                Constraint::Length(1),  // Status line
            ])
            .split(f.area());
        
        // Header
        self.render_header(f, chunks[0]);
        
        // Main content
        match self.current_view {
            View::Library => self.render_library(f, chunks[1]),
            View::Queue => self.render_queue(f, chunks[1]),
            View::Search => self.render_search(f, chunks[1]),
        }
        
        // Player controls
        self.render_player_controls(f, chunks[2]);
        
        // Status line
        self.render_status_line(f, chunks[3]);
    }
    
    fn render_header(&self, f: &mut Frame, area: Rect) {
        let title = Paragraph::new("♪ Dab Music Player")
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(title, area);
    }
    
    fn render_library(&mut self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self.tracks
            .iter()
            .map(|track| {
                ListItem::new(Line::from(vec![
                    Span::styled(&track.title, Style::default().fg(Color::White)),
                    Span::raw(" - "),
                    Span::styled(&track.artist, Style::default().fg(Color::Gray)),
                ]))
            })
            .collect();
        
        let list = List::new(items)
            .block(Block::default()
                .title("Library")
                .borders(Borders::ALL))
            .highlight_style(Style::default().bg(Color::DarkGray))
            .highlight_symbol("► ");
        
        f.render_stateful_widget(list, area, &mut self.list_state);
    }
    
    fn render_queue(&self, f: &mut Frame, area: Rect) {
        let queue_content = Paragraph::new("Queue view - Coming soon!")
            .block(Block::default()
                .title("Queue")
                .borders(Borders::ALL));
        f.render_widget(queue_content, area);
    }
    
    fn render_search(&self, f: &mut Frame, area: Rect) {
        let search_content = Paragraph::new("Search view - Coming soon!")
            .block(Block::default()
                .title("Search")
                .borders(Borders::ALL));
        f.render_widget(search_content, area);
    }
    
    fn render_player_controls(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(60),  // Track info
                Constraint::Percentage(40),  // Controls
            ])
            .split(area);
        
        // Track info (placeholder)
        let track_info = Paragraph::new("No track playing")
            .block(Block::default()
                .title("Now Playing")
                .borders(Borders::ALL));
        f.render_widget(track_info, chunks[0]);
        
        // Controls
        let controls = Paragraph::new("Space: Play/Pause | N: Next | P: Previous | Q: Quit")
            .block(Block::default()
                .title("Controls")
                .borders(Borders::ALL));
        f.render_widget(controls, chunks[1]);
    }
    
    fn render_status_line(&self, f: &mut Frame, area: Rect) {
        let status_text = if let Some(ref msg) = self.status_message {
            msg.clone()
        } else {
            format!("View: {:?} | {} tracks", self.current_view, self.tracks.len())
        };
        
        let status = Paragraph::new(status_text)
            .style(Style::default().fg(Color::Yellow));
        f.render_widget(status, area);
    }
    
    async fn handle_key_event(&mut self, key: KeyCode) -> DabResult<()> {
        match key {
            KeyCode::Char('q') => self.should_quit = true,
            
            // Navigation
            KeyCode::Up => self.list_up(),
            KeyCode::Down => self.list_down(),
            
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
            KeyCode::Enter => self.play_selected_track().await?,
            
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
            Some(i) => if i == 0 { self.tracks.len() - 1 } else { i - 1 },
            None => 0,
        };
        self.list_state.select(Some(i));
    }
    
    fn list_down(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => if i >= self.tracks.len() - 1 { 0 } else { i + 1 },
            None => 0,
        };
        self.list_state.select(Some(i));
    }
    
    async fn switch_view(&mut self, view: View) {
        if self.current_view != view {
            self.current_view = view;
            self.refresh_tracks_list().await;
            self.list_state.select(if self.tracks.is_empty() { None } else { Some(0) });
        }
    }
    
    async fn refresh_tracks_list(&mut self) {
        self.tracks = match self.current_view {
            View::Library => {
                // For now, just return empty list since we don't have tracks yet
                Vec::new()
            }
            View::Queue => {
                // TODO: Get tracks from player queue
                Vec::new()
            }
            View::Search => {
                // TODO: Implement search
                Vec::new()
            }
        };
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
            if let Some(track) = self.tracks.get(selected_index) {
                self.player.load_and_play(&track.url).await?;
            }
        }
        Ok(())
    }
}