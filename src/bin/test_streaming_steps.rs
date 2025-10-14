use futures_util::StreamExt;
use reqwest;
use rodio::{Decoder, OutputStream, Sink};
use serde_json;
use std::io::{BufReader, Cursor, Read, Write};
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

impl Read for Arc<Mutex<SimpleStreamBuffer>> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut retries = 0;
        let max_retries = 500; // 5 seconds total

        loop {
            let mut buffer = self.lock().unwrap();

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
    println!("1. Simple streaming (minimal buffer, start after 1MB)");
    println!("2. Aggressive streaming (start after 100KB)");
    println!("3. DAB-like streaming (circular buffer with underrun handling)");
    println!("Enter choice (1/2/3): ");

    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
    let choice = input.trim();

    match choice {
        "1" => test_simple_streaming(&stream_url, 1024 * 1024).await,  // 1MB buffer
        "2" => test_simple_streaming(&stream_url, 100 * 1024).await,    // 100KB buffer
        "3" => test_dab_like_streaming(&stream_url).await,
        _ => println!("Invalid choice"),
    }
}

async fn test_simple_streaming(url: &str, min_buffer_bytes: usize) {
    println!("\n🎵 Simple Streaming Test");
    println!("Min buffer before playback: {} KB\n", min_buffer_bytes / 1024);

    // 创建缓冲区（50MB容量）
    let buffer = SimpleStreamBuffer::new(50 * 1024 * 1024);
    let buffer_clone = Arc::clone(&buffer);

    // 启动下载任务
    let url_clone = url.to_string();
    let download_handle = tokio::spawn(async move {
        download_to_simple_buffer(&url_clone, buffer_clone).await;
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

    // 开始播放
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let sink = Sink::try_new(&stream_handle).unwrap();

    let buf_reader = BufReader::new(buffer.clone());

    let source = match Decoder::new(buf_reader) {
        Ok(s) => {
            println!("✅ Streaming decoder created\n");
            s
        }
        Err(e) => {
            println!("❌ Failed to create decoder: {}", e);
            download_handle.abort();
            return;
        }
    };

    sink.set_volume(0.8);
    sink.append(source);

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🎵 PLAYING (Simple Streaming)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Listen for pops/clicks!\n");

    // 监控播放和缓冲状态
    for i in 0..30 {
        if sink.empty() {
            println!("\nPlayback finished");
            break;
        }

        let buffer_lock = buffer.lock().unwrap();
        let bytes_buffered = buffer_lock.bytes_available();
        let is_complete = buffer_lock.complete;
        drop(buffer_lock);

        print!("\r⏱️  {} seconds | Buffer: {} KB {}",
               i + 1,
               bytes_buffered / 1024,
               if is_complete { "(download complete)" } else { "" });

        use std::io::{self};
        io::stdout().flush().unwrap();

        thread::sleep(Duration::from_secs(1));
    }

    println!("\n\nStopping...");
    sink.stop();
    download_handle.abort();
}

async fn test_dab_like_streaming(url: &str) {
    println!("\n🎵 DAB-like Streaming Test (with circular buffer simulation)");

    // 这里可以添加更复杂的循环缓冲区实现
    // 模拟DAB的StreamingAudioSource行为

    // 创建循环缓冲区
    let buffer = CircularStreamBuffer::new(10 * 1024 * 1024); // 10MB circular buffer
    let buffer_clone = Arc::clone(&buffer);

    // 启动下载
    let url_clone = url.to_string();
    let download_handle = tokio::spawn(async move {
        download_to_circular_buffer(&url_clone, buffer_clone).await;
    });

    // 等待初始数据
    println!("⏳ Waiting for initial buffer (500KB)...");
    loop {
        let ready = buffer.lock().unwrap().is_ready_for_playback(500 * 1024);
        if ready {
            println!("✅ Ready for playback\n");
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    // 播放
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let sink = Sink::try_new(&stream_handle).unwrap();

    let buf_reader = BufReader::with_capacity(64 * 1024, buffer.clone());

    let source = match Decoder::new(buf_reader) {
        Ok(s) => {
            println!("✅ Streaming decoder created (with circular buffer)\n");
            s
        }
        Err(e) => {
            println!("❌ Failed to create decoder: {}", e);
            download_handle.abort();
            return;
        }
    };

    sink.set_volume(0.8);
    sink.append(source);

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🎵 PLAYING (DAB-like with circular buffer)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("This simulates DAB's streaming behavior\n");

    for i in 0..30 {
        if sink.empty() {
            println!("\nPlayback finished");
            break;
        }

        let buffer_info = buffer.lock().unwrap();
        let bytes_buffered = buffer_info.bytes_available();
        let underruns = buffer_info.underrun_count;
        drop(buffer_info);

        print!("\r⏱️  {} seconds | Buffer: {} KB | Underruns: {}",
               i + 1,
               bytes_buffered / 1024,
               underruns);

        use std::io::{self};
        io::stdout().flush().unwrap();

        thread::sleep(Duration::from_secs(1));
    }

    println!("\n\nStopping...");
    sink.stop();
    download_handle.abort();
}

// 循环缓冲区实现（模拟DAB的StreamingAudioSource）
struct CircularStreamBuffer {
    data: Vec<u8>,
    capacity: usize,
    write_pos: usize,
    read_pos: usize,
    total_written: usize,
    total_read: usize,
    complete: bool,
    underrun_count: u32,
}

impl CircularStreamBuffer {
    fn new(capacity: usize) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(CircularStreamBuffer {
            data: vec![0; capacity],
            capacity,
            write_pos: 0,
            read_pos: 0,
            total_written: 0,
            total_read: 0,
            complete: false,
            underrun_count: 0,
        }))
    }

    fn write_data(&mut self, chunk: &[u8]) {
        for byte in chunk {
            self.data[self.write_pos] = *byte;
            self.write_pos = (self.write_pos + 1) % self.capacity;
            self.total_written += 1;
        }
    }

    fn bytes_available(&self) -> usize {
        self.total_written.saturating_sub(self.total_read)
    }

    fn is_ready_for_playback(&self, min_bytes: usize) -> bool {
        self.bytes_available() >= min_bytes || self.complete
    }
}

impl Read for Arc<Mutex<CircularStreamBuffer>> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut buffer = self.lock().unwrap();

        let available = buffer.bytes_available();

        if available == 0 && buffer.complete {
            return Ok(0); // EOF
        }

        if available == 0 {
            // 缓冲区欠载
            buffer.underrun_count += 1;

            // 等待数据，最多100ms
            drop(buffer);
            for _ in 0..10 {
                thread::sleep(Duration::from_millis(10));
                buffer = self.lock().unwrap();
                if buffer.bytes_available() > 0 {
                    break;
                }
                drop(buffer);
            }
            buffer = self.lock().unwrap();

            let available = buffer.bytes_available();
            if available == 0 {
                // 仍然没有数据，返回WouldBlock
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    format!("Buffer underrun #{}", buffer.underrun_count),
                ));
            }
        }

        let to_read = buf.len().min(available);
        for i in 0..to_read {
            buf[i] = buffer.data[buffer.read_pos];
            buffer.read_pos = (buffer.read_pos + 1) % buffer.capacity;
            buffer.total_read += 1;
        }

        Ok(to_read)
    }
}

async fn download_to_simple_buffer(url: &str, buffer: Arc<Mutex<SimpleStreamBuffer>>) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap();

    let response = client.get(url).send().await.unwrap();
    let content_length = response.content_length().unwrap_or(0);

    println!("📥 Downloading {} MB in background...", content_length / (1024 * 1024));

    let mut stream = response.bytes_stream();
    let mut downloaded = 0u64;

    while let Some(chunk_result) = stream.next().await {
        if let Ok(chunk) = chunk_result {
            let mut buf = buffer.lock().unwrap();
            buf.write_data(&chunk);
            drop(buf);

            downloaded += chunk.len() as u64;

            if downloaded % (1024 * 1024) == 0 {
                let mb = downloaded / (1024 * 1024);
                println!("📥 Downloaded {} MB", mb);
            }
        }
    }

    let mut buf = buffer.lock().unwrap();
    buf.complete = true;
    println!("✅ Download complete!");
}

async fn download_to_circular_buffer(url: &str, buffer: Arc<Mutex<CircularStreamBuffer>>) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap();

    let response = client.get(url).send().await.unwrap();
    let content_length = response.content_length().unwrap_or(0);

    println!("📥 Downloading {} MB with circular buffer...", content_length / (1024 * 1024));

    let mut stream = response.bytes_stream();
    let mut downloaded = 0u64;

    while let Some(chunk_result) = stream.next().await {
        if let Ok(chunk) = chunk_result {
            let mut buf = buffer.lock().unwrap();
            buf.write_data(&chunk);
            drop(buf);

            downloaded += chunk.len() as u64;
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