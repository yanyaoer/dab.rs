# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Tooling setup
You run in an environment where ast-grep is available; whenever a search requires syntax-aware or structural matching, default to ast-grep --lang rust -p '<pattern>' (or set --lang appropriately) and avoid falling back to text-only tools like rg or grep unless I explicitly request a plain-text search.

## Development Commands

### Building and Testing
```bash
# Build project (debug)
cargo build

# Build for release
cargo build --release

# Run tests
cargo test

# Run specific test
cargo test streaming

# Run with test output
cargo test -- --nocapture

# Format code (ALWAYS use before commits)
cargo fmt

# Lint code
cargo clippy

# Check for errors without building
cargo check
```

### Running the Application
```bash
# Start TUI interface
cargo run

# Run with specific commands
cargo run -- search "query"
cargo run -- play
cargo run -- pause
```

## Project Architecture

### High-Level Structure
DAB is a terminal-based music player with three main architectural layers:

1. **TUI Layer** (`src/tui/`): Ratatui-based terminal interface with event handling
2. **Player Engine** (`src/player/`): Asynchronous audio playback engine using Tokio channels
3. **API/Caching Layer** (`src/`): HTTP client for music API and local caching system

### Core Modules

#### Player Engine (`src/player/`)
- **Async Architecture**: Built on Tokio with channel-based communication
- **Streaming Support**: Real-time streaming with buffering via `streaming.rs` and `loader.rs`
- **Queue Management**: Full queue control with repeat modes in `queue.rs`
- **Audio Pipeline**: Symphonia decoder → Rodio sink pipeline in `engine.rs`
- **Download Manager**: Background downloading and caching in `download_manager.rs`

#### TUI System (`src/tui/`)
- **Event-driven**: Crossterm events with async handling in `handlers.rs`
- **Component Architecture**: Reusable UI components in `components.rs`
- **State Management**: Centralized app state in `app.rs`
- **Multiple Views**: Library, Queue, Search, Album Detail, Artist Discography

#### API Integration (`src/`)
- **HTTP Client**: Async reqwest-based client in `async_client.rs`
- **Caching**: Multi-layer caching (API responses + audio files) in `cache.rs` and `api_cache.rs`
- **Search**: Music API wrapper in `search.rs`
- **Models**: Track/Album/Artist models with OpenAPI schema compatibility

### Communication Patterns

#### Player Commands
The player uses Tokio channels for async communication:
```rust
// Player commands are sent via channels
PlayerCommand::LoadAndPlayTrack(track)
PlayerCommand::AddTrackToQueue(track)
PlayerCommand::Play/Pause/Stop
```

#### Event System
```rust
// Player events are broadcast to UI
PlayerEvent::StateChanged(PlayerState)
PlayerEvent::TrackChanged(Track)
PlayerEvent::DownloadProgress { track_id, progress }
```

#### TUI Key Bindings
All song lists support consistent key bindings:
- `Enter`: Play track/album
- `l`: Show album details
- `h`: Show artist discography  
- `a`: Add to queue next
- `A`: Replace queue with all tracks
- `m`: Add album to favorites library

### Data Flow

1. **Search Flow**: TUI → API client → Cache check → HTTP request → JSON parsing → Display
2. **Playback Flow**: Track selection → Stream URL fetch → Download manager → Audio decoder → Audio sink
3. **Caching Flow**: Stream download → Local storage → ID3 metadata extraction → Cache database

### Configuration

- **Config File**: `~/.config/dab/config.toml` (managed by `config.rs`)
- **Cache Directory**: `~/.cache/dab/` for audio files and metadata
- **Logging**: Structured logging to `/tmp/dab_rs.log` (NO print statements allowed)

## API Integration

### OpenAPI Schema
The project integrates with a music API defined in `resource/openapi.yaml`:
- **Search**: `/search?q=query&type=track|album|artist`
- **Album Details**: `/album?albumId=id`
- **Artist Discography**: `/discography?artistId=id`
- **Stream URLs**: `/stream?trackId=id`

### Mock Data for Testing
Test API responses are provided in `resource/`:
- `mock_search_q_coldplay_type_artist.json`
- `mock_discography_artistId_40226.json`
- `mock_album_albumId_0190295978044.json`

## Development Guidelines

### Code Style
- Always run `cargo fmt` before commits
- Use structured logging instead of print statements
- Follow async/await patterns consistently
- Handle errors with proper error types from `error.rs`

### Testing
- Unit tests for core functionality in `tests/`
- Integration tests for streaming and caching
- Mock data for API testing
- Test both success and error paths

### Audio Handling
- Use Symphonia for decoding multiple formats
- Implement proper buffering for streaming
- Support local and remote audio sources
- Handle stream URL expiration gracefully

### UI Development
- Component reusability across different views
- Consistent key binding patterns
- Proper async event handling
- Kitty graphics protocol for cover art display

### Caching Strategy
- API responses cached in memory with HTTP cache headers
- Audio files cached locally with ID3 metadata
- Cleanup based on cache size limits
- Preloading for better user experience

## Important Notes

- **No Print Statements**: All output must use structured logging
- **Async First**: All I/O operations should be async
- **Error Handling**: Use the custom error types, don't panic
- **ID Handling**: Convert all API IDs to strings for consistency
- **Stream URLs**: Always request fresh URLs before playback due to expiration
- **Memory Management**: Use circular buffers for streaming to prevent memory leaks