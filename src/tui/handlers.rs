use crate::async_client::AsyncNetworkClient;
use crate::cache::Cache;
use crate::error::DabResult;
use crate::library::Library;
use crate::player::Track;
use crate::search::DabAlbum;
use crate::tui::components::{ListItemType, UnifiedList};
use log::info;

pub enum NavigationAction {
    ShowAlbumDetail(String),
    ShowArtistDiscography(String),
    PlayTrack(Track),
    PlayAlbum(Vec<Track>),
    AddTrackNext(Track),
    AddAlbumNext(Vec<Track>),
    AddToFavorites(DabAlbum),
    ClearAndPlayAll(Vec<Track>), // Clear queue and add all tracks from current list
}

pub struct KeyHandler {
    cache: Option<Cache>,
    library: Option<Library>,
}

impl KeyHandler {
    pub fn new() -> Self {
        Self {
            cache: None,
            library: None,
        }
    }

    pub async fn new_with_cache_and_library() -> DabResult<Self> {
        let cache = Cache::new().await.ok();
        let library = if let Some(ref cache) = cache {
            Library::new(cache.clone()).await.ok()
        } else {
            None
        };

        Ok(Self { cache, library })
    }

    pub fn set_cache(&mut self, cache: Cache) {
        self.cache = Some(cache);
    }

    pub fn set_library(&mut self, library: Library) {
        self.library = Some(library);
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
                    // First, try to load tracks from local cache
                    let mut tracks_found = Vec::new();

                    if let Some(ref cache) = self.cache {
                        info!(
                            "Checking local cache for album tracks: {} by {}",
                            album.title, album.artist
                        );

                        // Try to get tracks from cached files by album name
                        if let Ok(cached_tracks) = cache.get_album_tracks(&album.title).await {
                            for cached_track in cached_tracks {
                                if let Some(metadata) = cached_track.metadata {
                                    tracks_found.push(Track {
                                        id: cached_track.track_id.clone(),
                                        title: metadata
                                            .title
                                            .unwrap_or_else(|| "Unknown".to_string()),
                                        artist: metadata
                                            .artist
                                            .unwrap_or_else(|| album.artist.clone()),
                                        album: metadata
                                            .album
                                            .unwrap_or_else(|| album.title.clone()),
                                        duration_ms: metadata.duration_ms.unwrap_or(0),
                                        local_path: Some(format!(
                                            "file://{}",
                                            cached_track.file_path.display()
                                        )),
                                        cover_url: album.cover.clone(),
                                        track_id: Some(cached_track.track_id),
                                        artist_id: album.artist_id.clone(),
                                        album_id: Some(album.id.clone()),
                                    });
                                }
                            }
                        }

                        if !tracks_found.is_empty() {
                            info!(
                                "Found {} tracks in local cache for album: {}",
                                tracks_found.len(),
                                album.title
                            );
                            return Ok(Some(NavigationAction::PlayAlbum(tracks_found)));
                        }
                    }

                    // If no local tracks found, try network (but this might fail if API is down)
                    info!(
                        "No cached tracks found for album: {}, attempting network request",
                        album.title
                    );
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
        let mut all_tracks = Vec::new();

        // Collect all tracks from the current list
        for item in &list.items {
            match item {
                ListItemType::Track(track) => {
                    // Skip album/artist entries in search results
                    if !track.title.starts_with("[Album]") && !track.title.starts_with("[Artist]") {
                        all_tracks.push(track.clone());
                    }
                }
                ListItemType::FavoriteAlbum(album) => {
                    // First try local cache
                    let mut cached_tracks_found = false;

                    if let Some(ref cache) = self.cache {
                        if let Ok(cached_tracks) = cache.get_album_tracks(&album.title).await {
                            for cached_track in cached_tracks {
                                if let Some(metadata) = cached_track.metadata {
                                    all_tracks.push(Track {
                                        id: cached_track.track_id.clone(),
                                        title: metadata
                                            .title
                                            .unwrap_or_else(|| "Unknown".to_string()),
                                        artist: metadata
                                            .artist
                                            .unwrap_or_else(|| album.artist.clone()),
                                        album: metadata
                                            .album
                                            .unwrap_or_else(|| album.title.clone()),
                                        duration_ms: metadata.duration_ms.unwrap_or(0),
                                        local_path: Some(format!(
                                            "file://{}",
                                            cached_track.file_path.display()
                                        )),
                                        cover_url: album.cover.clone(),
                                        track_id: Some(cached_track.track_id),
                                        artist_id: album.artist_id.clone(),
                                        album_id: Some(album.id.clone()),
                                    });
                                    cached_tracks_found = true;
                                }
                            }
                        }
                    }

                    // Only try network if no local tracks found
                    if !cached_tracks_found {
                        // Load album details and get all tracks
                        if let Ok(dab_album) = network_client.get_album(album.id.clone()).await {
                            if let Some(tracks) = &dab_album.tracks {
                                let player_tracks: Vec<Track> = tracks
                                    .iter()
                                    .map(|dab_track| Track::from_dab_track(dab_track))
                                    .collect();
                                all_tracks.extend(player_tracks);
                            }
                        }
                    }
                }
                ListItemType::DabAlbum(album) => {
                    if let Some(tracks) = &album.tracks {
                        let player_tracks: Vec<Track> = tracks
                            .iter()
                            .map(|dab_track| Track::from_dab_track(dab_track))
                            .collect();
                        all_tracks.extend(player_tracks);
                    }
                }
                ListItemType::QueueTrack { track, .. } => {
                    all_tracks.push(track.clone());
                }
                ListItemType::Album(_album) => {
                    // Library albums don't have IDs, can't load them
                    // Skip these for now
                }
            }
        }

        if !all_tracks.is_empty() {
            Ok(Some(NavigationAction::ClearAndPlayAll(all_tracks)))
        } else {
            Ok(None)
        }
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
