# DAB Music Player

A Unix-style command-line music player built with Rust, featuring streaming playback, local caching, and a terminal user interface inspired by cmus.

```
┌─ DAB Music Player ─────────────────────────────────────────────────────────────────────┐
│ ▶ Anthrax - Madhouse: The Very Best Of Anthrax - Madhouse (Album Version) [02:25/04:17] │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ Favorite Albums (16) │ Enter: Play │ l: Album detail │ h: Artist discography │ a: Add │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ Saxon - Strong Arm Of The Law (Edition spéciale) (Unknown year)                        │
│ Anthrax - Madhouse: The Very Best Of Anthrax                                          │
│ Megadeth - Rust In Peace (Unknown year)                                               │
│ Jacqueline du Pré - The Heart of the Cello (Unknown year)                            │
│ Antonio Vivaldi - Great Composers - Vivaldi (Unknown year)                            │
│ Billie Eilish - HIT ME HARD AND SOFT (Unknown year)                                   │
│ Sia - 1000 Forms Of Fear (Deluxe Version) (2014-07-04)                               │
│ Amorphis - Tuonela (Unknown year)                                                     │
│ Pantera - Cowboys From Hell (2010-04-17)                                              │
│ Judas Priest - Painkiller (1990-08-01)                                               │
│ Arch Enemy - Deceivers (2022-08-12)                                                   │
│ Opeth - The Last Will And Testament (2024-10-11)                                      │
│ My Dying Bride - 34.788%... Complete (1998-01-01)                                     │
│ Amon Amarth - The Great Heathen Army (2022-08-05)                                     │
│ Coldplay - X&Y (2005-06-06)                                                           │
│ Lana Del Rey - Born To Die (2012-01-30)                                               │
└────────────────────────────────────────────────────────────────────────────────────────┘
Player state: Playing
```

## Features

### 🎵 Core Playback
- **Streaming Audio**: Stream music directly from online sources with adaptive buffering
- **Local Caching**: Automatic caching of streamed tracks for offline playback
- **Format Support**: FLAC, MP3, and other formats via Symphonia audio decoder
- **Queue Management**: Full queue control with repeat modes and shuffle

### 🔍 Music Discovery
- **Online Search**: Search for tracks, albums, and artists using `/` key
- **Artist Discography**: Browse complete artist discographies with `h` key
- **Album Details**: View detailed album information with `l` key
- **Smart Caching**: Intelligent caching based on listening patterns

### 📚 Library Management
- **Favorite Albums**: Mark albums as favorites with `m` key for quick access
- **Local Library**: Browse cached music organized by artist and album
- **Metadata Support**: Full ID3 tag support with cover art display

### 🎨 Terminal Interface
- **cmus-inspired**: Familiar interface for cmus users
- **Cover Art**: Display album covers using Kitty graphics protocol
- **Audio Visualization**: TUI-style audio visualization below cover art
- **Responsive Design**: Adapts to terminal size with scrolling text support

### ⚡ Performance
- **Async Architecture**: Non-blocking playback engine using Tokio
- **Smart Buffering**: Adaptive buffering based on network conditions
- **Preloading**: Intelligent preloading of upcoming tracks
- **Memory Efficient**: Circular buffer management for streaming

## Installation

### Prerequisites
- Rust 1.70+ 
- Audio system (ALSA/PulseAudio on Linux, CoreAudio on macOS)

### Build from Source
```bash
git clone https://github.com/yourusername/dab.rs.git
cd dab.rs
cargo build --release
```

### Install
```bash
cargo install --path .
```

## Usage

### Command Line Interface
```bash
# Start TUI
dab

# Search for music
dab search "Metallica"

# Play/pause
dab play
dab pause

# Queue management
dab queue "https://example.com/song.mp3"
dab next
dab prev
```

### TUI Controls

| Key | Action |
|-----|--------|
| `Enter` | Play selected track/album |
| `/` | Search for music |
| `j`/`k` | Navigate up/down |
| `l` | Show album details |
| `h` | Show artist discography |
| `a` | Add track to queue (next) |
| `A` | Replace queue with current view |
| `m` | Add album to favorites |
| `Esc` | Go back to previous view |
| `Space` | Play/pause |
| `n` | Next track |
| `p` | Previous track |

## Configuration

Configuration is stored in `~/.config/dab/config.toml`:

```toml
[audio]
volume = 0.8
cache_dir = "~/.cache/dab"
max_cache_size_gb = 10

[network]
timeout_ms = 5000
retry_attempts = 3

[ui]
show_cover_art = true
audio_visualization = true
```

## API Integration

DAB integrates with music services through a RESTful API. See `resource/openapi.yaml` for the complete API specification.

### Supported Endpoints
- `/search` - Search for tracks, albums, artists
- `/album` - Get album details and tracks
- `/discography` - Get artist's complete discography
- `/stream` - Get streaming URLs for tracks
- `/download` - Download album information

## Architecture

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   TUI Layer     │    │  Player Engine  │    │  Cache System   │
│                 │    │                 │    │                 │
│ • Event Loop    │◄──►│ • Audio Sink    │◄──►│ • Local Storage │
│ • Key Bindings  │    │ • Queue Mgmt    │    │ • Metadata DB   │
│ • UI Rendering  │    │ • Stream Buffer │    │ • Cleanup Logic │
└─────────────────┘    └─────────────────┘    └─────────────────┘
         │                       │                       │
         │              ┌─────────────────┐              │
         └─────────────►│  Search API     │◄─────────────┘
                        │                 │
                        │ • Music Search  │
                        │ • Metadata      │
                        │ • Stream URLs   │
                        └─────────────────┘
```

## Development

### Building
```bash
# Debug build
cargo build

# Run tests
cargo test

# Format code
cargo fmt

# Lint
cargo clippy
```

### Logging
Logs are written to `/tmp/dab_rs.log` with configurable levels:
- `ERROR`: Critical errors
- `WARN`: Warnings and recoverable errors  
- `INFO`: General information
- `DEBUG`: Detailed debugging information

### Testing
```bash
# Run all tests
cargo test

# Run specific test
cargo test streaming

# Run with output
cargo test -- --nocapture
```

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

### Code Style
- Use `cargo fmt` for formatting
- Follow Rust naming conventions
- Add tests for new functionality
- Update documentation as needed

## License

This project is licensed under the GNU Affero General Public License v3.0 - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- Inspired by [cmus](https://cmus.github.io/) terminal music player
- Built with [Symphonia](https://github.com/pdeljanov/Symphonia) audio decoder
- UI powered by [Ratatui](https://github.com/ratatui-org/ratatui)
- Audio playback via [Rodio](https://github.com/RustAudio/rodio)

## Support

- 📚 [Documentation](docs/)
- 🐛 [Issue Tracker](https://github.com/yourusername/dab.rs/issues)
- 💬 [Discussions](https://github.com/yourusername/dab.rs/discussions)