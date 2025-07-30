use crate::cache::Cache;
use crate::error::DabResult;
use crate::library::Library;
use crate::player::PlayerEngine;
use crate::search::MusicSearchApi;
use crate::tui::TuiApp;
use clap::{Parser, Subcommand};
use log::{error, info};

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
    /// Search for music
    Search {
        /// Search query
        query: String,
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

        match self.command {
            Some(Commands::Play { track }) => {
                if let Some(track_id) = track {
                    info!("Loading and playing track: {}", track_id);
                    player.load_and_play(&track_id).await?;
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
            }
            Some(Commands::Search { query }) => {
                info!("Performing search for query: {}", query);
                let search_api = MusicSearchApi::new();

                match search_api.search_tracks(&query, 10).await {
                    Ok(tracks) => {
                        info!(
                            "Search completed successfully, found {} results",
                            tracks.len()
                        );

                        // For CLI output, we need to write to stdout, but also log the search
                        eprintln!("Search results for '{}' ({} results):", query, tracks.len());
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

                            if !track.url.is_empty() {
                                eprintln!("   Stream URL: {}", track.url);
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
                        error!("Search failed for query '{}': {}", query, e);
                        eprintln!("Search failed: {}", e);
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
                let mut tui = TuiApp::new(player, library).await?;
                tui.run().await?;
            }
            None => {
                info!("No command specified, starting TUI interface");
                // Default to TUI if no command specified
                let mut tui = TuiApp::new(player, library).await?;
                tui.run().await?;
            }
        }

        Ok(())
    }
}
