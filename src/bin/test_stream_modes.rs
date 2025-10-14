use futures_util::StreamExt;
use reqwest;
use rodio::{Decoder, OutputStream, Sink};
use serde::{Deserialize, Serialize};
use std::io::{BufReader, Cursor, Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Debug, Deserialize, Serialize)]
struct TrackStreamResponse {
    #[serde(rename = "trackId")]
    track_id: Option<String>,
    quality: Option<String>,
    #[serde(rename = "originalTrackUrl")]
    original_track_url: Option<String>,
    error: Option<String>,
}

// 简单的流式缓冲区
struct StreamBuffer {
    data: Vec<u8>,
    write_pos: usize,
    read_pos: usize,
    complete: bool,
}

impl StreamBuffer {
    fn new() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(StreamBuffer {
            data: Vec::new(),
            write_pos: 0,
            read_pos: 0,
            complete: false,
        }))
    }
}

impl Read for Arc<Mutex<StreamBuffer>> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut buffer = self.lock().unwrap();

        // 等待一些数据
        let mut retries = 0;
        while buffer.read_pos >= buffer.write_pos && !buffer.complete {
            drop(buffer);
            thread::sleep(Duration::from_millis(10));
            buffer = self.lock().unwrap();

            retries += 1;
            if retries > 500 { // 5秒超时
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "Timeout waiting for data",
                ));
            }
        }

        let available = buffer.write_pos - buffer.read_pos;
        if available == 0 && buffer.complete {
            return Ok(0); // EOF
        }

        let to_read = buf.len().min(available);
        buf[..to_read].copy_from_slice(&buffer.data[buffer.read_pos..buffer.read_pos + to_read]);
        buffer.read_pos += to_read;

        Ok(to_read)
    }
}

#[tokio::main]
async fn main() {
    println!("DAB.rs Progressive Streaming Test");
    println!("==================================\n");

    // Step 1: 获取stream URL
    let stream_url = match get_stream_url().await {
        Ok(url) => {
            println!("✅ Stream URL obtained: {}\n", url);
            url
        }
        Err(e) => {
            println!("❌ Failed to get stream URL: {}", e);
            return;
        }
    };

    println!("Select test mode:");
    println!("1. Download complete then play (like streaming_buffer=0)");
    println!("2. Progressive streaming (start playing while downloading)");
    println!("Enter choice (1 or 2): ");

    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
    let choice = input.trim();

    match choice {
        "1" => test_download_then_play(&stream_url).await,
        "2" => test_progressive_streaming(&stream_url).await,
        _ => println!("Invalid choice"),
    }
}

async fn get_stream_url() -> Result<String, Box<dyn std::error::Error>> {
    println!("🔍 Requesting stream URL from Squid API...");

    let url = "https://kraken.squid.wtf/track/?id=15200216&quality=LOSSLESS";
    println!("   URL: {}", url);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(format!("API returned status: {}", response.status()).into());
    }

    let stream_response: TrackStreamResponse = response.json().await?;

    if let Some(error) = stream_response.error {
        return Err(format!("API error: {}", error).into());
    }

    stream_response.original_track_url
        .ok_or_else(|| "No stream URL in response".into())
}

async fn test_download_then_play(url: &str) {
    println!("\n📥 Mode 1: Download complete file then play...\n");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap();

    let mut response = client.get(url).send().await.unwrap();
    let content_length = response.content_length().unwrap_or(0);

    println!("Downloading {} bytes...", content_length);

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

    println!("\n✅ Download complete!\n");

    // 播放
    play_from_memory(audio_data);
}

async fn test_progressive_streaming(url: &str) {
    println!("\n🎵 Mode 2: Progressive streaming (play while downloading)...\n");

    let buffer = StreamBuffer::new();
    let buffer_clone = Arc::clone(&buffer);

    // 启动下载任务
    let url_clone = url.to_string();
    let download_handle = tokio::spawn(async move {
        download_to_buffer(&url_clone, buffer_clone).await;
    });

    // 等待一些初始数据
    println!("Buffering initial data...");
    thread::sleep(Duration::from_secs(2));

    // 开始播放
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let sink = Sink::try_new(&stream_handle).unwrap();

    // 创建decoder从流式buffer
    let buf_reader = BufReader::new(buffer);
    let source = match Decoder::new(buf_reader) {
        Ok(s) => {
            println!("✅ Streaming decoder created\n");
            s
        }
        Err(e) => {
            println!("❌ Failed to create decoder: {}", e);
            return;
        }
    };

    sink.set_volume(0.8);
    sink.append(source);

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🎵 PLAYING (Progressive Streaming)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Listen for pops/clicks during streaming!\n");

    // 播放30秒
    for i in 0..30 {
        if sink.empty() {
            println!("\nPlayback finished");
            break;
        }

        print!("\r⏱️  {} seconds elapsed", i + 1);
        use std::io::{self};
        io::stdout().flush().unwrap();

        thread::sleep(Duration::from_secs(1));
    }

    println!("\n\nStopping...");
    sink.stop();
    download_handle.abort();
}

async fn download_to_buffer(url: &str, buffer: Arc<Mutex<StreamBuffer>>) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap();

    let response = client.get(url).send().await.unwrap();
    let content_length = response.content_length().unwrap_or(0);

    println!("Starting download of {} bytes...", content_length);

    let mut stream = response.bytes_stream();
    let mut downloaded = 0u64;

    while let Some(chunk_result) = stream.next().await {
        if let Ok(chunk) = chunk_result {
            let mut buf = buffer.lock().unwrap();
            buf.data.extend_from_slice(&chunk);
            buf.write_pos = buf.data.len();

            downloaded += chunk.len() as u64;

            if downloaded % (1024 * 1024) == 0 { // 每MB打印一次
                let mb = downloaded / (1024 * 1024);
                println!("Downloaded {} MB", mb);
            }
        }
    }

    let mut buf = buffer.lock().unwrap();
    buf.complete = true;
    println!("Download complete!");
}

fn play_from_memory(audio_data: Vec<u8>) {
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let sink = Sink::try_new(&stream_handle).unwrap();

    let cursor = Cursor::new(audio_data);
    let buf_reader = BufReader::new(cursor);
    let source = Decoder::new(buf_reader).unwrap();

    sink.set_volume(0.8);
    sink.append(source);

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🎵 PLAYING (From memory)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    for i in 0..30 {
        if sink.empty() {
            break;
        }
        print!("\r⏱️  {} seconds elapsed", i + 1);
        use std::io::{self};
        io::stdout().flush().unwrap();
        thread::sleep(Duration::from_secs(1));
    }

    println!("\n✅ Test completed!");
}