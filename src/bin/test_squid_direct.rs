use reqwest;
use rodio::{Decoder, OutputStream, Sink};
use serde_json;
use std::io::{BufReader, Cursor};
use std::time::Duration;

#[tokio::main]
async fn main() {
    println!("DAB.rs Direct Stream Playback Test");
    println!("===================================\n");

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
            // 首先尝试主要字段
            for key in PRIMARY_KEYS.iter() {
                if let Some(serde_json::Value::String(s)) = map.get(*key) {
                    if looks_like_stream_url(s) {
                        return Some(s.clone());
                    }
                }
            }

            // 尝试 "url" 字段
            if let Some(serde_json::Value::String(url)) = map.get("url") {
                if looks_like_stream_url(url) {
                    return Some(url.clone());
                }
            }

            // 递归搜索其他字段
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
        serde_json::Value::Array(entries) => {
            // 从数组中查找URL（反向遍历，优先选择最后的）
            for entry in entries.iter().rev() {
                if let Some(url) = extract_stream_url(entry) {
                    return Some(url);
                }
            }
            None
        }
        _ => None,
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

    let body = response.text().await?;
    println!("   Response length: {} bytes", body.len());

    // 尝试解析JSON
    if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&body) {
        // 打印JSON结构用于调试
        if let Some(obj) = json_value.as_object() {
            println!("   Response fields: {:?}", obj.keys().collect::<Vec<_>>());
        }

        // 使用extract_stream_url方法来查找URL
        if let Some(stream_url) = extract_stream_url(&json_value) {
            return Ok(stream_url);
        }

        // 如果没找到，尝试直接作为字符串
        if let Some(url_str) = json_value.as_str() {
            if url_str.starts_with("http") {
                return Ok(url_str.to_string());
            }
        }
    }

    // 最后尝试直接将body作为URL
    let trimmed = body.trim().trim_matches('"');
    if trimmed.starts_with("http") {
        Ok(trimmed.to_string())
    } else {
        Err(format!("Could not extract stream URL from response: {}",
            if body.len() > 200 { &body[..200] } else { &body }).into())
    }
}

async fn download_audio(url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()?;

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()).into());
    }

    let content_length = response.content_length().unwrap_or(0);
    println!("   Content-Length: {} bytes", content_length);

    let mut audio_data = Vec::new();
    let mut downloaded = 0u64;

    let mut response = response;
    while let Some(chunk) = response.chunk().await? {
        audio_data.extend_from_slice(&chunk);
        downloaded += chunk.len() as u64;

        // 显示下载进度
        if content_length > 0 {
            let progress = (downloaded as f64 / content_length as f64) * 100.0;
            print!("\r   Progress: {:.1}% ({}/{} bytes)", progress, downloaded, content_length);
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
    println!("  • If HAS pops: Problem might be in the stream itself or rodio");
}