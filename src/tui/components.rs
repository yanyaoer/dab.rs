use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};

use crate::library::{Album, FavoriteAlbum};
use crate::player::Track;
use crate::search::{DabAlbum, DabTrack};

#[derive(Debug, Clone)]
pub enum ListItemType {
    Track(Track),
    Album(Album),
    DabAlbum(DabAlbum),
    FavoriteAlbum(FavoriteAlbum),
    QueueTrack { track: Track, is_current: bool },
}

impl ListItemType {
    pub fn get_track(&self) -> Option<&Track> {
        match self {
            ListItemType::Track(track) => Some(track),
            ListItemType::QueueTrack { track, .. } => Some(track),
            _ => None,
        }
    }

    pub fn get_album(&self) -> Option<&Album> {
        match self {
            ListItemType::Album(album) => Some(album),
            _ => None,
        }
    }

    pub fn get_favorite_album(&self) -> Option<&FavoriteAlbum> {
        match self {
            ListItemType::FavoriteAlbum(album) => Some(album),
            _ => None,
        }
    }

    pub fn get_dab_album(&self) -> Option<&DabAlbum> {
        match self {
            ListItemType::DabAlbum(album) => Some(album),
            _ => None,
        }
    }

    pub fn get_artist_name(&self) -> String {
        match self {
            ListItemType::Track(track) => track.artist.clone(),
            ListItemType::Album(album) => album.artist.clone(),
            ListItemType::DabAlbum(album) => album.artist.clone(),
            ListItemType::FavoriteAlbum(album) => album.artist.clone(),
            ListItemType::QueueTrack { track, .. } => track.artist.clone(),
        }
    }

    pub fn get_title(&self) -> String {
        match self {
            ListItemType::Track(track) => track.title.clone(),
            ListItemType::Album(album) => album.title.clone(),
            ListItemType::DabAlbum(album) => album.title.clone(),
            ListItemType::FavoriteAlbum(album) => album.title.clone(),
            ListItemType::QueueTrack { track, .. } => track.title.clone(),
        }
    }

    pub fn get_album_title(&self) -> String {
        match self {
            ListItemType::Track(track) => track.album.clone(),
            ListItemType::Album(album) => album.title.clone(),
            ListItemType::DabAlbum(album) => album.title.clone(),
            ListItemType::FavoriteAlbum(album) => album.title.clone(),
            ListItemType::QueueTrack { track, .. } => track.album.clone(),
        }
    }

    pub fn get_artist_id(&self) -> Option<String> {
        match self {
            ListItemType::Track(track) => track.artist_id.clone(),
            ListItemType::Album(_album) => None, // Library Album doesn't have artist_id
            ListItemType::DabAlbum(album) => album.artist_id.clone(),
            ListItemType::FavoriteAlbum(album) => album.artist_id.clone(),
            ListItemType::QueueTrack { track, .. } => track.artist_id.clone(),
        }
    }

    pub fn get_album_id(&self) -> Option<String> {
        match self {
            ListItemType::Track(track) => track.album_id.clone(),
            ListItemType::Album(_album) => None, // Library Album doesn't have id
            ListItemType::DabAlbum(album) => Some(album.id.clone()),
            ListItemType::FavoriteAlbum(album) => Some(album.id.clone()),
            ListItemType::QueueTrack { track, .. } => track.album_id.clone(),
        }
    }

    pub fn is_local(&self) -> bool {
        match self {
            ListItemType::Track(track) => track.is_local(),
            ListItemType::QueueTrack { track, .. } => track.is_local(),
            _ => false,
        }
    }
}

pub struct UnifiedList {
    pub items: Vec<ListItemType>,
    pub state: ListState,
    pub title: String,
    pub show_help: bool,
}

impl UnifiedList {
    pub fn new(title: String) -> Self {
        Self {
            items: Vec::new(),
            state: ListState::default(),
            title,
            show_help: false,
        }
    }

    pub fn with_tracks(mut self, tracks: Vec<Track>) -> Self {
        self.items = tracks.into_iter().map(ListItemType::Track).collect();
        self.reset_selection();
        self
    }

    pub fn with_albums(mut self, albums: Vec<Album>) -> Self {
        self.items = albums.into_iter().map(ListItemType::Album).collect();
        self.reset_selection();
        self
    }

    pub fn with_favorite_albums(mut self, albums: Vec<&FavoriteAlbum>) -> Self {
        self.items = albums
            .into_iter()
            .map(|album| ListItemType::FavoriteAlbum(album.clone()))
            .collect();
        self.reset_selection();
        self
    }

    pub fn with_dab_albums(mut self, albums: Vec<DabAlbum>) -> Self {
        self.items = albums.into_iter().map(ListItemType::DabAlbum).collect();
        self.reset_selection();
        self
    }

    pub fn with_queue_tracks(mut self, tracks: Vec<Track>, current_index: Option<usize>) -> Self {
        self.items = tracks
            .into_iter()
            .enumerate()
            .map(|(i, track)| ListItemType::QueueTrack {
                track,
                is_current: current_index == Some(i),
            })
            .collect();
        self.reset_selection();
        self
    }

    pub fn with_help(mut self, show_help: bool) -> Self {
        self.show_help = show_help;
        self
    }

    pub fn reset_selection(&mut self) {
        self.state
            .select(if self.items.is_empty() { None } else { Some(0) });
    }

    pub fn move_up(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    if self.items.is_empty() {
                        0
                    } else {
                        self.items.len() - 1
                    }
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn move_down(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if self.items.is_empty() || i >= self.items.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn get_selected_item(&self) -> Option<&ListItemType> {
        if let Some(selected) = self.state.selected() {
            self.items.get(selected)
        } else {
            None
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        let list_items: Vec<ListItem> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| Self::create_list_item_static(i, item))
            .collect();

        let help_text = if self.show_help {
            " | Enter: Play | l: Album detail | h: Artist discography | a: Add next | A: Play all | m: Add to favorites"
        } else {
            ""
        };

        let title = format!("{} ({}){}", self.title, self.items.len(), help_text);

        let list = List::new(list_items)
            .block(Block::default().title(title).borders(Borders::ALL))
            .highlight_style(Style::default().bg(Color::DarkGray));
        // .highlight_symbol("► ");

        f.render_stateful_widget(list, area, &mut self.state);
    }

    fn create_list_item_static(index: usize, item: &ListItemType) -> ListItem {
        match item {
            ListItemType::Track(track) => {
                let cached_indicator = if track.is_local() { " [local]" } else { "" };
                ListItem::new(Line::from(vec![
                    Span::styled(&track.artist, Style::default().fg(Color::Cyan)),
                    Span::raw(" - "),
                    Span::styled(&track.album, Style::default().fg(Color::Blue)),
                    Span::raw(" - "),
                    Span::styled(&track.title, Style::default().fg(Color::White)),
                    Span::styled(cached_indicator, Style::default().fg(Color::Green)),
                ]))
            }
            ListItemType::Album(album) => ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{} - {}", album.artist, album.title),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!(
                        " ({})",
                        album
                            .year
                            .map(|y| y.to_string())
                            .as_deref()
                            .unwrap_or("Unknown year")
                    ),
                    Style::default().fg(Color::DarkGray),
                ),
            ])),
            ListItemType::FavoriteAlbum(album) => {
                let release_year = album
                    .release_date
                    .as_ref()
                    .and_then(|date| date.split('-').next())
                    .unwrap_or("Unknown");

                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:<30}", album.title),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(" - ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{:<20}", album.artist),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        format!(
                            " ({}, {} tracks)",
                            release_year,
                            album.track_count.unwrap_or(0)
                        ),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
            }
            ListItemType::DabAlbum(album) => {
                let release_year = album
                    .release_date
                    .as_ref()
                    .and_then(|date| date.split('-').next())
                    .unwrap_or("Unknown");

                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:<30}", album.title),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(" - ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{:<20}", album.artist),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        format!(
                            " ({}, {} tracks)",
                            release_year,
                            album.track_count.unwrap_or(0)
                        ),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
            }
            ListItemType::QueueTrack { track, is_current } => {
                let prefix = if *is_current { "▶ " } else { "  " };
                let cached_indicator = if track.is_local() { " [local]" } else { "" };

                let style = if *is_current {
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{}{}. ", prefix, index + 1),
                        Style::default().fg(Color::Gray),
                    ),
                    Span::styled(&track.title, style),
                    Span::raw(" - "),
                    Span::styled(&track.album, Style::default().fg(Color::Blue)),
                    Span::raw(" - "),
                    Span::styled(&track.artist, Style::default().fg(Color::Yellow)),
                    Span::styled(cached_indicator, Style::default().fg(Color::Green)),
                ]))
            }
        }
    }
}

pub fn render_track_list_with_index<'a>(tracks: &'a [Track], _title: &str) -> Vec<ListItem<'a>> {
    tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let cached_indicator = if track.is_local() { " [local]" } else { "" };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{}. ", i + 1), Style::default().fg(Color::Gray)),
                Span::styled(&track.title, Style::default().fg(Color::White)),
                Span::styled(cached_indicator, Style::default().fg(Color::Green)),
            ]))
        })
        .collect()
}

pub fn render_dab_track_list_with_index<'a>(
    tracks: &'a [DabTrack],
    _title: &str,
) -> Vec<ListItem<'a>> {
    tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let duration = track
                .duration
                .map(|d| format!(" ({}:{:02})", d / 60, d % 60))
                .unwrap_or_default();
            ListItem::new(format!("{}. {}{}", i + 1, track.title, duration))
        })
        .collect()
}
