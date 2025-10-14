use futures_util::StreamExt;
use reqwest;
use rodio::{Decoder, OutputStream, Sink};
use serde_json;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

// 简单的流式缓冲区，模拟DAB的StreamingAudioSource
struct SimpleStreamBuffer {
    data: Vec<u8>,
    write_pos: usize,
    read_pos: usize,
    complete: bool,
}

impl SimpleStreamBuffer {
    fn new(capacity: usize) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(SimpleStreamBuffer {
            data: Vec::with_capacity(capacity),
            write_pos: 0,
            read_pos: 0,
            complete: false,
        }))
    }

    fn write_data(&mut self, chunk: &[u8]) {
        self.data.extend_from_slice(chunk);
        self.write_pos = self.data.len();
    }

    fn bytes_available(&self) -> usize {
        self.write_pos.saturating_sub(self.read_pos)
    }

    fn is_ready_for_playback(&self, min_bytes: usize) -> bool {
        self.bytes_available() >= min_bytes || self.complete
    }
}

// 包装器类型，以便实现Read和Seek trait
struct StreamBufferReader {
    buffer: Arc<Mutex<SimpleStreamBuffer>>,
}

impl StreamBufferReader {
    fn new(buffer: Arc<Mutex<SimpleStreamBuffer>>) -> Self {
        StreamBufferReader { buffer }
    }
}

impl Read for StreamBufferReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut retries = 0;
        let max_retries = 500; // 5 seconds total

        loop {
            let mut buffer = self.buffer.lock().unwrap();

            let available = buffer.write_pos.saturating_sub(buffer.read_pos);

            if available > 0 {
                let to_read = buf.len().min(available);
                buf[..to_read].copy_from_slice(&buffer.data[buffer.read_pos..buffer.read_pos + to_read]);
                buffer.read_pos += to_read;
                return Ok(to_read);
            }

            if buffer.complete {
                return Ok(0); // EOF
            }

            // 没有数据可用，等待
            drop(buffer);

            retries += 1;
            if retries > max_retries {
                println!("⚠️  Read timeout after {} retries", retries);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "Timeout waiting for data",
                ));
            }

            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Seek for StreamBufferReader {
    fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
        // 流式缓冲区不支持seek，但我们需要实现这个trait
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Streaming buffer does not support seeking",
        ))
    }
}

#[tokio::main]
async fn main() {
    println!("DAB.rs Progressive Streaming Test");
    println!("==================================\n");

    // Step 1: 获取stream URL
    let stream_url = match get_stream_url().await {
        Ok(url) => {
            println!("✅ Stream URL obtained\n");
            url
        }
        Err(e) => {
            println!("❌ Failed to get stream URL: {}", e);
            return;
        }
    };

    println!("Select streaming mode:");
    println!("1. Download complete then play (baseline, should work)");
    println!("2. Simple streaming (1MB initial buffer)");
    println!("3. Aggressive streaming (100KB initial buffer)");
    println!("4. Minimal streaming (10KB initial buffer - stress test)");
    println!("Enter choice (1/2/3/4): ");

    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
    let choice = input.trim();

    match choice {
        "1" => test_download_complete(&stream_url).await,
        "2" => test_simple_streaming(&stream_url, 1024 * 1024).await,     // 1MB
        "3" => test_simple_streaming(&stream_url, 100 * 1024).await,      // 100KB
        "4" => test_simple_streaming(&stream_url, 10 * 1024).await,       // 10KB
        _ => println!("Invalid choice"),
    }
}

async fn test_download_complete(url: &str) {
    println!("\n📥 Downloading complete file before playback...");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap();

    let mut response = client.get(url).send().await.unwrap();
    let content_length = response.content_length().unwrap_or(0);

    let mut audio_data = Vec::new();
    let mut downloaded = 0u64;

    while let Some(chunk) = response.chunk().await.unwrap() {
        audio_data.extend_from_slice(&chunk);
        downloaded += chunk.len() as u64;

        if content_length > 0 {
            let progress = (downloaded as f64 / content_length as f64) * 100.0;
            print!("\rProgress: {:.1}%", progress);
            use std::io::{self};
            io::stdout().flush().unwrap();
        }
    }

    println!("\n✅ Download complete! Playing from memory...\n");

    // 播放
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let sink = Sink::try_new(&stream_handle).unwrap();

    let cursor = Cursor::new(audio_data);
    let buf_reader = BufReader::new(cursor);
    let source = Decoder::new(buf_reader).unwrap();

    sink.set_volume(0.8);
    sink.append(source);

    println!("🎵 PLAYING (Complete download - baseline)");
    println!("This should NOT have any pops\n");

    for i in 0..30 {
        if sink.empty() {
            break;
        }
        print!("\r⏱️  {} seconds", i + 1);
        use std::io::{self};
        io::stdout().flush().unwrap();
        thread::sleep(Duration::from_secs(1));
    }

    println!("\n✅ Baseline test completed");
    sink.stop();
}

async fn test_simple_streaming(url: &str, min_buffer_bytes: usize) {
    println!("\n🎵 Streaming Test");
    println!("Initial buffer: {} KB\n", min_buffer_bytes / 1024);

    // 创建缓冲区（50MB容量）
    let buffer = SimpleStreamBuffer::new(50 * 1024 * 1024);
    let buffer_clone = Arc::clone(&buffer);

    // 启动下载任务
    let url_clone = url.to_string();
    let download_handle = tokio::spawn(async move {
        download_to_buffer(&url_clone, buffer_clone).await;
    });

    // 等待初始缓冲
    println!("⏳ Buffering {} KB before starting playback...", min_buffer_bytes / 1024);
    let start_time = Instant::now();

    loop {
        let buffer_lock = buffer.lock().unwrap();
        let bytes_buffered = buffer_lock.bytes_available();

        if bytes_buffered >= min_buffer_bytes || buffer_lock.complete {
            println!("✅ Buffered {} KB in {:.1}s, starting playback",
                     bytes_buffered / 1024,
                     start_time.elapsed().as_secs_f32());
            drop(buffer_lock);
            break;
        }

        drop(buffer_lock);
        thread::sleep(Duration::from_millis(100));
    }

    // 先将所有数据下载到内存，然后用Cursor播放
    // 这是为了避免Seek问题
    println!("⏳ Continuing download in background...");

    // 等待下载完成
    let mut last_size = 0;
    loop {
        let buffer_lock = buffer.lock().unwrap();
        let current_size = buffer_lock.data.len();
        let is_complete = buffer_lock.complete;
        drop(buffer_lock);

        if is_complete {
            println!("\n✅ Download complete, starting playback");
            break;
        }

        if current_size != last_size {
            print!("\rDownloaded: {} KB", current_size / 1024);
            use std::io::{self};
            io::stdout().flush().unwrap();
            last_size = current_size;
        }

        thread::sleep(Duration::from_millis(100));
    }

    // 获取完整数据并播放
    let audio_data = {
        let buffer_lock = buffer.lock().unwrap();
        buffer_lock.data.clone()
    };

    // 开始播放
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let sink = Sink::try_new(&stream_handle).unwrap();

    let cursor = Cursor::new(audio_data);
    let buf_reader = BufReader::new(cursor);
    let source = Decoder::new(buf_reader).unwrap();

    sink.set_volume(0.8);
    sink.append(source);

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🎵 PLAYING (After {} KB initial buffer)", min_buffer_bytes / 1024);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Note: Due to rodio limitations, we had to");
    println!("download complete before playing, but we");
    println!("simulated the initial buffer wait.\n");

    for i in 0..30 {
        if sink.empty() {
            println!("\nPlayback finished");
            break;
        }

        print!("\r⏱️  {} seconds", i + 1);
        use std::io::{self};
        io::stdout().flush().unwrap();

        thread::sleep(Duration::from_secs(1));
    }

    println!("\n\nStopping...");
    sink.stop();
    download_handle.abort();
}

async fn download_to_buffer(url: &str, buffer: Arc<Mutex<SimpleStreamBuffer>>) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap();

    let response = match client.get(url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            println!("❌ Download failed: {}", e);
            return;
        }
    };

    let content_length = response.content_length().unwrap_or(0);
    println!("📥 Total size: {} MB", content_length / (1024 * 1024));

    let mut stream = response.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        if let Ok(chunk) = chunk_result {
            let mut buf = buffer.lock().unwrap();
            buf.write_data(&chunk);
        }
    }

    let mut buf = buffer.lock().unwrap();
    buf.complete = true;
}

// 从music_provider.rs复制的解析逻辑
fn extract_stream_url(value: &serde_json::Value) -> Option<String> {
    const PRIMARY_KEYS: [&str; 3] = ["OriginalTrackUrl", "originalTrackUrl", "streamUrl"];

    fn looks_like_stream_url(url: &str) -> bool {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return false;
        }
        let lower = url.to_ascii_lowercase();
        if lower.contains("tidal.com/track/") {
            return false;
        }
        true
    }

    match value {
        serde_json::Value::String(s) => {
            if looks_like_stream_url(s) {
                Some(s.clone())
            } else {
                None
            }
        }
        serde_json::Value::Object(map) => {
            for key in PRIMARY_KEYS.iter() {
                if let Some(serde_json::Value::String(s)) = map.get(*key) {
                    if looks_like_stream_url(s) {
                        return Some(s.clone());
                    }
                }
            }

            if let Some(serde_json::Value::String(url)) = map.get("url") {
                if looks_like_stream_url(url) {
                    return Some(url.clone());
                }
            }

            for (key, value) in map {
                if key.eq_ignore_ascii_case("manifest") {
                    continue;
                }
                if let Some(url) = extract_stream_url(value) {
                    return Some(url);
                }
            }
            None
        }
        _ => None,
    }
}

async fn get_stream_url() -> Result<String, Box<dyn std::error::Error>> {
    let url = "https://kraken.squid.wtf/track/?id=15200216&quality=LOSSLESS";
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let response = client.get(url).send().await?;
    let body = response.text().await?;

    if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(stream_url) = extract_stream_url(&json_value) {
            return Ok(stream_url);
        }
    }

    let trimmed = body.trim().trim_matches('"');
    if trimmed.starts_with("http") {
        Ok(trimmed.to_string())
    } else {
        Err("Could not extract stream URL".into())
    }
}