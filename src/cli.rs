use clap::{Parser, Subcommand};
use crate::error::DabResult;
use crate::player::PlayerEngine;
use crate::library::Library;
use crate::cache::Cache;
use crate::tui::TuiApp;

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
                    player.load_and_play(&track_id).await?;
                } else {
                    player.play().await?;
                }
            }
            Some(Commands::Pause) => {
                player.pause().await?;
            }
            Some(Commands::Resume) => {
                player.resume().await?;
            }
            Some(Commands::Stop) => {
                player.stop().await?;
            }
            Some(Commands::Next) => {
                player.next().await?;
            }
            Some(Commands::Prev) => {
                player.previous().await?;
            }
            Some(Commands::Queue { url }) => {
                player.add_to_queue(&url).await?;
            }
            Some(Commands::Status) => {
                let status = player.get_status().await?;
                println!("{}", serde_json::to_string_pretty(&status)?);
            }
            Some(Commands::Tui) => {
                let mut tui = TuiApp::new(player, library).await?;
                tui.run().await?;
            }
            None => {
                // Default to TUI if no command specified
                let mut tui = TuiApp::new(player, library).await?;
                tui.run().await?;
            }
        }
        
        Ok(())
    }
}