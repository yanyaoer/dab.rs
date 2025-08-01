# DAB 音乐播放器

一个用 Rust 构建的 Unix 风格命令行音乐播放器，具有流式播放、本地缓存和受 cmus 启发的终端用户界面。

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

## 功能特性

### 🎵 核心播放
- **流式音频**：直接从在线源流式播放音乐，具有自适应缓冲
- **本地缓存**：自动缓存流式播放的曲目，支持离线播放
- **格式支持**：通过 Symphonia 音频解码器支持 FLAC、MP3 等格式
- **队列管理**：完整的队列控制，支持重复模式和随机播放

### 🔍 音乐发现
- **在线搜索**：使用 `/` 键搜索曲目、专辑和艺术家
- **艺术家专辑集**：使用 `h` 键浏览完整的艺术家专辑集
- **专辑详情**：使用 `l` 键查看详细的专辑信息
- **智能缓存**：基于收听模式的智能缓存

### 📚 音乐库管理
- **收藏专辑**：使用 `m` 键将专辑标记为收藏，便于快速访问
- **本地音乐库**：按艺术家和专辑组织浏览缓存的音乐
- **元数据支持**：完整的 ID3 标签支持，包含专辑封面显示

### 🎨 终端界面
- **cmus 风格**：为 cmus 用户提供熟悉的界面
- **专辑封面**：使用 Kitty 图形协议显示专辑封面
- **音频可视化**：封面下方的 TUI 风格音频可视化效果
- **响应式设计**：适应终端大小，支持文本滚动

### ⚡ 性能优化
- **异步架构**：使用 Tokio 的非阻塞播放引擎
- **智能缓冲**：基于网络条件的自适应缓冲
- **预加载**：智能预加载即将播放的曲目
- **内存高效**：流式播放的循环缓冲区管理

## 安装

### 系统要求
- Rust 1.70+ 
- 音频系统（Linux 上的 ALSA/PulseAudio，macOS 上的 CoreAudio）

### 从源码构建
```bash
git clone https://github.com/yourusername/dab.rs.git
cd dab.rs
cargo build --release
```

### 安装
```bash
cargo install --path .
```

## 使用方法

### 命令行界面
```bash
# 启动 TUI
dab

# 搜索音乐
dab search "Metallica"

# 播放/暂停
dab play
dab pause

# 队列管理
dab queue "https://example.com/song.mp3"
dab next
dab prev
```

### TUI 控制

| 按键 | 操作 |
|-----|--------|
| `Enter` | 播放选中的曲目/专辑 |
| `/` | 搜索音乐 |
| `j`/`k` | 上/下导航 |
| `l` | 显示专辑详情 |
| `h` | 显示艺术家专辑集 |
| `a` | 将曲目添加到队列（下一首） |
| `A` | 用当前视图替换队列 |
| `m` | 将专辑添加到收藏 |
| `Esc` | 返回上一个视图 |
| `Space` | 播放/暂停 |
| `n` | 下一首 |
| `p` | 上一首 |

## 配置

配置文件存储在 `~/.config/dab/config.toml`：

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

## API 集成

DAB 通过 RESTful API 与音乐服务集成。完整的 API 规范请参见 `resource/openapi.yaml`。

### 支持的端点
- `/search` - 搜索曲目、专辑、艺术家
- `/album` - 获取专辑详情和曲目
- `/discography` - 获取艺术家的完整专辑集
- `/stream` - 获取曲目的流式 URL
- `/download` - 下载专辑信息

## 架构

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   TUI 层        │    │  播放引擎       │    │  缓存系统       │
│                 │    │                 │    │                 │
│ • 事件循环      │◄──►│ • 音频输出      │◄──►│ • 本地存储      │
│ • 按键绑定      │    │ • 队列管理      │    │ • 元数据数据库  │
│ • UI 渲染       │    │ • 流缓冲        │    │ • 清理逻辑      │
└─────────────────┘    └─────────────────┘    └─────────────────┘
         │                       │                       │
         │              ┌─────────────────┐              │
         └─────────────►│  搜索 API       │◄─────────────┘
                        │                 │
                        │ • 音乐搜索      │
                        │ • 元数据        │
                        │ • 流式 URL      │
                        └─────────────────┘
```

## 开发

### 构建
```bash
# 调试构建
cargo build

# 运行测试
cargo test

# 格式化代码
cargo fmt

# 代码检查
cargo clippy
```

### 日志记录
日志记录到 `/tmp/dab_rs.log`，支持可配置的日志级别：
- `ERROR`：严重错误
- `WARN`：警告和可恢复错误  
- `INFO`：一般信息
- `DEBUG`：详细调试信息

### 测试
```bash
# 运行所有测试
cargo test

# 运行特定测试
cargo test streaming

# 带输出运行
cargo test -- --nocapture
```

## 贡献

1. Fork 仓库
2. 创建功能分支 (`git checkout -b feature/amazing-feature`)
3. 提交更改 (`git commit -m 'Add amazing feature'`)
4. 推送到分支 (`git push origin feature/amazing-feature`)
5. 开启 Pull Request

### 代码风格
- 使用 `cargo fmt` 格式化代码
- 遵循 Rust 命名约定
- 为新功能添加测试
- 根据需要更新文档

## 许可证

本项目采用 GNU Affero General Public License v3.0 许可证 - 详见 [LICENSE](LICENSE) 文件。

## 致谢

- 受 [cmus](https://cmus.github.io/) 终端音乐播放器启发
- 使用 [Symphonia](https://github.com/pdeljanov/Symphonia) 音频解码器构建
- UI 基于 [Ratatui](https://github.com/ratatui-org/ratatui)
- 音频播放通过 [Rodio](https://github.com/RustAudio/rodio)

## 支持

- 📚 [文档](docs/)
- 🐛 [问题跟踪](https://github.com/yourusername/dab.rs/issues)
- 💬 [讨论](https://github.com/yourusername/dab.rs/discussions)