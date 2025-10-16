use reqwest;
use rodio::{Decoder, OutputStream, Sink};
use serde::{Deserialize, Serialize};
use std::io::{BufReader, Cursor, Read};
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

#[tokio::main]
async fn main() {
    println!("DAB.rs Stream URL Direct Playback Test");
    println!("=======================================\n");

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

    // Step 2: 下载音频数据到内存（无缓冲，无缓存）
    println!("📥 Downloading audio data to memory (no buffer, no cache)...");
    let audio_data = match download_audio(&stream_url).await {
        Ok(data) => {
            let size_mb = data.len() as f64 / (1024.0 * 1024.0);
            println!("✅ Downloaded {:.2} MB of audio data\n", size_mb);
            data
        }
        Err(e) => {
            println!("❌ Failed to download audio: {}", e);
            return;
        }
    };

    // Step 3: 直接播放内存中的音频数据
    println!("🎵 Playing audio directly from memory...");
    play_audio_from_memory(audio_data);
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

    stream_response
        .original_track_url
        .ok_or_else(|| "No stream URL in response".into())
}

async fn download_audio(url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()?;

    let mut response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()).into());
    }

    let content_length = response.content_length().unwrap_or(0);
    println!("   Content-Length: {} bytes", content_length);

    let mut audio_data = Vec::new();
    let mut downloaded = 0u64;

    while let Some(chunk) = response.chunk().await? {
        audio_data.extend_from_slice(&chunk);
        downloaded += chunk.len() as u64;

        // 显示下载进度
        if content_length > 0 {
            let progress = (downloaded as f64 / content_length as f64) * 100.0;
            print!(
                "\r   Progress: {:.1}% ({}/{} bytes)",
                progress, downloaded, content_length
            );
            use std::io::{self, Write};
            io::stdout().flush().unwrap();
        }
    }

    println!("\n");

    Ok(audio_data)
}

fn play_audio_from_memory(audio_data: Vec<u8>) {
    // 创建音频输出
    let (_stream, stream_handle) = match OutputStream::try_default() {
        Ok(s) => {
            println!("✅ Audio output initialized");
            s
        }
        Err(e) => {
            println!("❌ Failed to initialize audio: {}", e);
            return;
        }
    };

    // 创建sink
    let sink = match Sink::try_new(&stream_handle) {
        Ok(s) => {
            println!("✅ Audio sink created");
            s
        }
        Err(e) => {
            println!("❌ Failed to create sink: {}", e);
            return;
        }
    };

    // 从内存数据创建decoder
    let cursor = Cursor::new(audio_data);
    let buf_reader = BufReader::new(cursor);

    let source = match Decoder::new(buf_reader) {
        Ok(s) => {
            println!("✅ Audio decoder created\n");
            s
        }
        Err(e) => {
            println!("❌ Failed to create decoder: {}", e);
            return;
        }
    };

    // 设置音量并播放
    sink.set_volume(0.8);
    sink.append(source);

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🎵 PLAYING (Direct from memory, no buffer/cache)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("");
    println!("Listen for:");
    println!("  • Any pops or clicks");
    println!("  • Audio quality issues");
    println!("  • Playback interruptions");
    println!("");
    println!("Playing for 30 seconds (Ctrl+C to stop)...\n");

    // 播放30秒
    for i in 0..30 {
        if sink.empty() {
            println!("\nPlayback finished (track ended)");
            break;
        }

        print!("\r⏱️  {} seconds elapsed", i + 1);
        use std::io::{self, Write};
        io::stdout().flush().unwrap();

        std::thread::sleep(Duration::from_secs(1));
    }

    println!("\n\nStopping playback...");
    sink.stop();

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("✅ Test completed!");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("\n🔍 Results:");
    println!("  • If NO pops: Problem is in DAB's streaming/buffering");
    println!("  • If HAS pops: Problem might be in the stream itself");
}
