use crate::async_client::AsyncNetworkClient;
use crate::error::DabResult;
use crate::player::Track;
use crate::search::DabAlbum;
use crate::tui::components::{ListItemType, UnifiedList};

pub enum NavigationAction {
    ShowAlbumDetail(String),
    ShowArtistDiscography(String),
    PlayTrack(Track),
    PlayAlbum(Vec<Track>),
    AddTrackNext(Track),
    AddAlbumNext(Vec<Track>),
    AddToFavorites(DabAlbum),
}

pub struct KeyHandler;

impl KeyHandler {
    pub fn new() -> Self {
        Self
    }

    pub async fn handle_enter(
        &self,
        list: &UnifiedList,
        network_client: &AsyncNetworkClient,
    ) -> DabResult<Option<NavigationAction>> {
        if let Some(item) = list.get_selected_item() {
            match item {
                ListItemType::Track(track) => {
                    return Ok(Some(NavigationAction::PlayTrack(track.clone())));
                }
                ListItemType::Album(_album) => {
                    // Library albums don't have IDs, can't load them
                    return Ok(None);
                }
                ListItemType::FavoriteAlbum(album) => {
                    // Load album details and play all tracks
                    if let Ok(dab_album) = network_client.get_album(album.id.clone()).await {
                        if let Some(tracks) = &dab_album.tracks {
                            let player_tracks: Vec<Track> = tracks
                                .iter()
                                .map(|dab_track| Track::from_dab_track(dab_track))
                                .collect();
                            return Ok(Some(NavigationAction::PlayAlbum(player_tracks)));
                        }
                    }
                }
                ListItemType::DabAlbum(album) => {
                    if let Some(tracks) = &album.tracks {
                        let player_tracks: Vec<Track> = tracks
                            .iter()
                            .map(|dab_track| Track::from_dab_track(dab_track))
                            .collect();
                        return Ok(Some(NavigationAction::PlayAlbum(player_tracks)));
                    }
                }
                ListItemType::QueueTrack { track, .. } => {
                    return Ok(Some(NavigationAction::PlayTrack(track.clone())));
                }
            }
        }
        Ok(None)
    }

    pub async fn handle_l_key(
        &self,
        list: &UnifiedList,
        _network_client: &AsyncNetworkClient,
    ) -> DabResult<Option<NavigationAction>> {
        if let Some(item) = list.get_selected_item() {
            if let Some(album_id) = item.get_album_id() {
                return Ok(Some(NavigationAction::ShowAlbumDetail(album_id)));
            }
        }
        Ok(None)
    }

    pub async fn handle_h_key(
        &self,
        list: &UnifiedList,
        network_client: &AsyncNetworkClient,
    ) -> DabResult<Option<NavigationAction>> {
        if let Some(item) = list.get_selected_item() {
            // Try to get artist_id from the item
            if let Some(artist_id) = item.get_artist_id() {
                return Ok(Some(NavigationAction::ShowArtistDiscography(artist_id)));
            } else {
                // If no artist_id, search for the artist by name
                let artist_name = item.get_artist_name();
                if let Ok(search_result) = network_client
                    .search(artist_name.clone(), "artist".to_string(), 10)
                    .await
                {
                    for search_item in search_result.get_results() {
                        if let crate::search::SearchResultItem::Artist(artist) = search_item {
                            if artist.name.to_lowercase() == artist_name.to_lowercase() {
                                return Ok(Some(NavigationAction::ShowArtistDiscography(
                                    artist.id.clone(),
                                )));
                            }
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    pub async fn handle_a_key(
        &self,
        list: &UnifiedList,
        _network_client: &AsyncNetworkClient,
    ) -> DabResult<Option<NavigationAction>> {
        if let Some(item) = list.get_selected_item() {
            if let Some(track) = item.get_track() {
                return Ok(Some(NavigationAction::AddTrackNext(track.clone())));
            }
        }
        Ok(None)
    }

    pub async fn handle_shift_a_key(
        &self,
        list: &UnifiedList,
        network_client: &AsyncNetworkClient,
    ) -> DabResult<Option<NavigationAction>> {
        if let Some(item) = list.get_selected_item() {
            match item {
                ListItemType::Track(track) => {
                    // For a single track, add just that track
                    return Ok(Some(NavigationAction::AddTrackNext(track.clone())));
                }
                ListItemType::Album(_album) => {
                    // Library albums don't have IDs, can't load them
                    return Ok(None);
                }
                ListItemType::FavoriteAlbum(album) => {
                    // Load album details and add all tracks
                    if let Ok(dab_album) = network_client.get_album(album.id.clone()).await {
                        if let Some(tracks) = &dab_album.tracks {
                            let player_tracks: Vec<Track> = tracks
                                .iter()
                                .map(|dab_track| Track::from_dab_track(dab_track))
                                .collect();
                            return Ok(Some(NavigationAction::AddAlbumNext(player_tracks)));
                        }
                    }
                }
                ListItemType::DabAlbum(album) => {
                    if let Some(tracks) = &album.tracks {
                        let player_tracks: Vec<Track> = tracks
                            .iter()
                            .map(|dab_track| Track::from_dab_track(dab_track))
                            .collect();
                        return Ok(Some(NavigationAction::AddAlbumNext(player_tracks)));
                    }
                }
                ListItemType::QueueTrack { track, .. } => {
                    return Ok(Some(NavigationAction::AddTrackNext(track.clone())));
                }
            }
        }
        Ok(None)
    }

    pub async fn handle_m_key(
        &self,
        list: &UnifiedList,
        _network_client: &AsyncNetworkClient,
    ) -> DabResult<Option<NavigationAction>> {
        if let Some(item) = list.get_selected_item() {
            match item {
                ListItemType::Track(track) => {
                    // Try to create a DabAlbum from track info for favorites
                    if let Some(album_id) = &track.album_id {
                        let album = DabAlbum {
                            id: album_id.clone(),
                            title: track.album.clone(),
                            artist: track.artist.clone(),
                            artist_id: track.artist_id.clone(),
                            release_date: None,
                            genre: None,
                            cover: track.cover_url.clone(),
                            tracks: None,
                            track_count: None,
                            duration: None,
                            label: None,
                            upc: None,
                            url: None,
                            streamable: None,
                            downloadable: None,
                            media_count: None,
                            maximum_channel_count: None,
                            parental_warning: None,
                            popularity: None,
                            audio_quality: None,
                        };
                        return Ok(Some(NavigationAction::AddToFavorites(album)));
                    }
                }
                ListItemType::Album(_album) => {
                    // Library albums can't be added to favorites without ID
                    return Ok(None);
                }
                ListItemType::FavoriteAlbum(album) => {
                    // Convert FavoriteAlbum to DabAlbum
                    let dab_album = DabAlbum {
                        id: album.id.clone(),
                        title: album.title.clone(),
                        artist: album.artist.clone(),
                        artist_id: album.artist_id.clone(),
                        release_date: album.release_date.clone(),
                        genre: None,
                        cover: album.cover.clone(),
                        tracks: None,
                        track_count: None,
                        duration: None,
                        label: None,
                        upc: None,
                        url: None,
                        streamable: None,
                        downloadable: None,
                        media_count: None,
                        maximum_channel_count: None,
                        parental_warning: None,
                        popularity: None,
                        audio_quality: None,
                    };
                    return Ok(Some(NavigationAction::AddToFavorites(dab_album)));
                }
                ListItemType::DabAlbum(album) => {
                    return Ok(Some(NavigationAction::AddToFavorites(album.clone())));
                }
                ListItemType::QueueTrack { track, .. } => {
                    // Same as track handling
                    if let Some(album_id) = &track.album_id {
                        let album = DabAlbum {
                            id: album_id.clone(),
                            title: track.album.clone(),
                            artist: track.artist.clone(),
                            artist_id: track.artist_id.clone(),
                            release_date: None,
                            genre: None,
                            cover: track.cover_url.clone(),
                            tracks: None,
                            track_count: None,
                            duration: None,
                            label: None,
                            upc: None,
                            url: None,
                            streamable: None,
                            downloadable: None,
                            media_count: None,
                            maximum_channel_count: None,
                            parental_warning: None,
                            popularity: None,
                            audio_quality: None,
                        };
                        return Ok(Some(NavigationAction::AddToFavorites(album)));
                    }
                }
            }
        }
        Ok(None)
    }
}
