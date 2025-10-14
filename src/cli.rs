use crate::async_client::{AsyncClient, AsyncNetworkClient, NetworkManager};
use crate::cache::Cache;
use crate::error::DabResult;
use crate::library::Library;
use crate::player::PlayerEngine;
use crate::tui::TuiApp;
use clap::{Parser, Subcommand};
use log::{error, info};
use std::time::Duration;

#[derive(Parser)]
#[command(name = "dab")]
#[command(about = "A unix-style command line music player")]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Play music
    Play {
        /// Track ID or URL to play
        track: Option<String>,
    },
    /// Pause playback
    Pause,
    /// Resume playback
    Resume,
    /// Stop playback
    Stop,
    /// Skip to next track
    Next,
    /// Skip to previous track
    Prev,
    /// Add track to queue
    Queue {
        /// URL or file path to add to queue
        url: String,
    },
    /// Clear the queue
    ClearQueue,
    /// Show current queue
    ShowQueue,
    /// Search for music
    Search {
        /// Search query
        query: String,
        /// Type of content to search for (track, album, artist)
        #[arg(short, long, default_value = "track")]
        r#type: String,
        /// Number of results to return (1-50)
        #[arg(short, long, default_value = "20")]
        limit: u32,
    },
    /// Show current status
    Status,
    /// Launch TUI interface
    Tui,
}

impl Cli {
    pub fn parse() -> Self {
        Parser::parse()
    }

    pub async fn run(self) -> DabResult<()> {
        let cache = Cache::new().await?;
        let library = Library::new(cache.clone()).await?;
        let mut player = PlayerEngine::new().await?;

        // Setup async network client
        let async_client = AsyncClient::new();
        let (network_manager, command_tx) = NetworkManager::new(async_client);
        let network_client = AsyncNetworkClient::new(command_tx);

        // Start background network manager
        let _network_task = tokio::spawn(network_manager.run());

        match self.command {
            Some(Commands::Play { track }) => {
                if let Some(track_id) = track {
                    info!("Loading and playing track: {}", track_id);
                    player.load_and_play(&track_id).await?;

                    if let Ok(wait_value) = std::env::var("DAB_DEBUG_WAIT_SECS") {
                        if let Ok(wait_secs) = wait_value.parse::<u64>() {
                            tokio::time::sleep(Duration::from_secs(wait_secs)).await;
                        }
                    }
                } else {
                    info!("Starting playback");
                    player.play().await?;
                }
            }
            Some(Commands::Pause) => {
                info!("Pausing playback");
                player.pause().await?;
            }
            Some(Commands::Resume) => {
                info!("Resuming playback");
                player.resume().await?;
            }
            Some(Commands::Stop) => {
                info!("Stopping playback");
                player.stop().await?;
            }
            Some(Commands::Next) => {
                info!("Skipping to next track");
                player.next().await?;
            }
            Some(Commands::Prev) => {
                info!("Skipping to previous track");
                player.previous().await?;
            }
            Some(Commands::Queue { url }) => {
                info!("Adding track to queue: {}", url);
                player.add_to_queue(&url).await?;
                println!("Added to queue: {}", url);
            }
            Some(Commands::ClearQueue) => {
                info!("Clearing queue");
                let queue = player.get_queue();
                queue.clear().await;
                println!("Queue cleared");
            }
            Some(Commands::ShowQueue) => {
                info!("Showing current queue");
                let queue = player.get_queue();
                let tracks = queue.get_queue().await;
                let current_index = queue.get_current_index().await;

                if tracks.is_empty() {
                    println!("Queue is empty");
                } else {
                    println!("Current Queue ({} tracks):", tracks.len());
                    if let Some(current) = current_index {
                        println!("Current playing: #{}", current + 1);
                    }
                    println!();

                    for (i, track) in tracks.iter().enumerate() {
                        let is_current = current_index == Some(i);
                        let prefix = if is_current { "▶ " } else { "  " };

                        println!(
                            "{}{}. {} - {} ({})",
                            prefix,
                            i + 1,
                            track.artist,
                            track.title,
                            track.album
                        );

                        if track.duration_ms > 0 {
                            let total_seconds = track.duration_ms / 1000;
                            let minutes = total_seconds / 60;
                            let seconds = total_seconds % 60;
                            println!("     Duration: {}:{:02}", minutes, seconds);
                        }
                    }
                }
            }
            Some(Commands::Search {
                query,
                r#type,
                limit,
            }) => {
                info!(
                    "Performing search for query: {} (type: {}, limit: {})",
                    query, r#type, limit
                );

                // Validate search type
                let search_type = r#type.as_str();
                if !["track", "album", "artist"].contains(&search_type) {
                    error!("Invalid search type: {}", search_type);
                    eprintln!(
                        "Error: Invalid search type '{}'. Must be one of: track, album, artist",
                        search_type
                    );
                    return Ok(());
                }

                // Validate limit
                if limit < 1 || limit > 50 {
                    error!("Invalid limit: {}", limit);
                    eprintln!("Error: Limit must be between 1 and 50");
                    return Ok(());
                }

                match search_type {
                    "track" => {
                        match network_client
                            .search(query.clone(), "track".to_string(), limit)
                            .await
                        {
                            Ok(search_result) => {
                                let mut tracks = Vec::new();
                                for item in search_result.get_results() {
                                    if let crate::search::SearchResultItem::Track(dab_track) = item
                                    {
                                        tracks
                                            .push(crate::player::Track::from_dab_track(&dab_track));
                                    }
                                }

                                info!(
                                    "Track search completed successfully, found {} results",
                                    tracks.len()
                                );

                                // For CLI output, we need to write to stdout, but also log the search
                                eprintln!(
                                    "Track search results for '{}' ({} results):",
                                    query,
                                    tracks.len()
                                );
                                eprintln!();

                                for (index, track) in tracks.iter().enumerate() {
                                    eprintln!(
                                        "{}. {} - {} ({})",
                                        index + 1,
                                        track.artist,
                                        track.title,
                                        track.album
                                    );

                                    if track.duration_ms > 0 {
                                        let total_seconds = track.duration_ms / 1000;
                                        let minutes = total_seconds / 60;
                                        let seconds = total_seconds % 60;
                                        eprintln!("   Duration: {}:{:02}", minutes, seconds);
                                    }

                                    // Show track metadata for online sources
                                    if track.requires_stream_url() {
                                        eprintln!(
                                            "   Track ID: {}",
                                            track
                                                .track_id
                                                .as_ref()
                                                .unwrap_or(&"Unknown".to_string())
                                        );
                                    } else if track.is_local() {
                                        eprintln!(
                                            "   Local file: {}",
                                            track
                                                .local_path
                                                .as_ref()
                                                .unwrap_or(&"Unknown".to_string())
                                        );
                                    }

                                    if let Some(cover_url) = &track.cover_url {
                                        eprintln!("   Cover: {}", cover_url);
                                    }

                                    eprintln!();
                                }

                                if !tracks.is_empty() {
                                    eprintln!("To play a track, use: dab play <Stream URL>");
                                    eprintln!("To add to queue, use: dab queue <Stream URL>");
                                }
                            }
                            Err(e) => {
                                error!("Track search failed for query '{}': {}", query, e);
                                eprintln!("Track search failed: {}", e);
                            }
                        }
                    }
                    "album" => match network_client
                        .search(query.clone(), "album".to_string(), limit)
                        .await
                    {
                        Ok(search_result) => {
                            let mut albums = Vec::new();
                            for item in search_result.get_results() {
                                if let crate::search::SearchResultItem::Album(album) = item {
                                    albums.push(album);
                                }
                            }

                            info!(
                                "Album search completed successfully, found {} results",
                                albums.len()
                            );

                            eprintln!(
                                "Album search results for '{}' ({} results):",
                                query,
                                albums.len()
                            );
                            eprintln!();

                            for (index, album) in albums.iter().enumerate() {
                                eprintln!("{}. {} - {}", index + 1, album.artist, album.title);

                                if let Some(release_date) = &album.release_date {
                                    eprintln!("   Release Date: {}", release_date);
                                }

                                if let Some(track_count) = album.track_count {
                                    eprintln!("   Tracks: {}", track_count);
                                }

                                if let Some(duration) = album.duration {
                                    let minutes = duration / 60;
                                    let seconds = duration % 60;
                                    eprintln!("   Duration: {}:{:02}", minutes, seconds);
                                }

                                if let Some(cover) = &album.cover {
                                    eprintln!("   Cover: {}", cover);
                                }

                                eprintln!();
                            }
                        }
                        Err(e) => {
                            error!("Album search failed for query '{}': {}", query, e);
                            eprintln!("Album search failed: {}", e);
                        }
                    },
                    "artist" => match network_client
                        .search(query.clone(), "artist".to_string(), limit)
                        .await
                    {
                        Ok(search_result) => {
                            let mut artists = Vec::new();
                            for item in search_result.get_results() {
                                if let crate::search::SearchResultItem::Artist(artist) = item {
                                    artists.push(artist);
                                }
                            }

                            info!(
                                "Artist search completed successfully, found {} results",
                                artists.len()
                            );

                            eprintln!(
                                "Artist search results for '{}' ({} results):",
                                query,
                                artists.len()
                            );
                            eprintln!();

                            for (index, artist) in artists.iter().enumerate() {
                                eprintln!("{}. {}", index + 1, artist.name);

                                if let Some(albums_count) = artist.albums_count {
                                    eprintln!("   Albums: {}", albums_count);
                                }

                                if let Some(image) = &artist.image {
                                    if let Some(large_image) = &image.large {
                                        eprintln!("   Image: {}", large_image);
                                    }
                                }

                                if let Some(biography) = &artist.biography {
                                    eprintln!("   Biography: {}", biography);
                                }

                                eprintln!();
                            }
                        }
                        Err(e) => {
                            error!("Artist search failed for query '{}': {}", query, e);
                            eprintln!("Artist search failed: {}", e);
                        }
                    },
                    _ => {
                        // This should never happen due to validation above
                        error!("Unsupported search type: {}", search_type);
                        eprintln!("Error: Unsupported search type '{}'", search_type);
                    }
                }
            }
            Some(Commands::Status) => {
                info!("Getting player status");
                match player.get_status().await {
                    Ok(status) => match serde_json::to_string_pretty(&status) {
                        Ok(json_output) => {
                            info!("Player status retrieved successfully");
                            eprintln!("{}", json_output);
                        }
                        Err(e) => {
                            error!("Failed to serialize player status: {}", e);
                            eprintln!("Error formatting status: {}", e);
                        }
                    },
                    Err(e) => {
                        error!("Failed to get player status: {}", e);
                        eprintln!("Failed to get status: {}", e);
                    }
                }
            }
            Some(Commands::Tui) => {
                info!("Starting TUI interface");
                let mut tui = TuiApp::new(player, library, network_client).await?;
                tui.run().await?;
            }
            None => {
                info!("No command specified, starting TUI interface");
                // Default to TUI if no command specified
                let mut tui = TuiApp::new(player, library, network_client).await?;
                tui.run().await?;
            }
        }

        Ok(())
    }
}
