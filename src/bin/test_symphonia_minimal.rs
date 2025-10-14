use futures_util::StreamExt;
use reqwest;
use rodio::{OutputStream, Sink, Source};
use serde_json;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// 极简的缓冲区 - 只是一个简单的Vec
struct MinimalBuffer {
    data: Vec<u8>,
    read_pos: usize,
    complete: bool,
}

impl MinimalBuffer {
    fn new() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(MinimalBuffer {
            data: Vec::with_capacity(10 * 1024 * 1024), // 10MB capacity
            read_pos: 0,
            complete: false,
        }))
    }
}

/// 极简的Reader - 避免复杂的重试逻辑
struct MinimalReader {
    buffer: Arc<Mutex<MinimalBuffer>>,
}

impl MinimalReader {
    fn new(buffer: Arc<Mutex<MinimalBuffer>>) -> Self {
        MinimalReader { buffer }
    }
}

impl Read for MinimalReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // 简单的读取策略：如果没有数据，等待更长时间
        let mut wait_count = 0;

        loop {
            // 尝试获取锁并读取
            if let Ok(mut buffer) = self.buffer.lock() {
                let available = buffer.data.len() - buffer.read_pos;

                if available > 0 {
                    // 有数据可读
                    let to_read = buf.len().min(available);
                    buf[..to_read].copy_from_slice(
                        &buffer.data[buffer.read_pos..buffer.read_pos + to_read]
                    );
                    buffer.read_pos += to_read;
                    return Ok(to_read);
                }

                // 没有数据，检查是否完成
                if buffer.complete && available == 0 {
                    return Ok(0); // EOF
                }

                // 释放锁
                drop(buffer);
            }

            // 等待数据，使用更长的延迟避免CPU占用
            wait_count += 1;

            // 最多等待10秒
            if wait_count > 200 {
                // 返回0而不是错误，让播放器处理
                println!("⚠️  Read timeout after 10 seconds, returning empty");
                return Ok(0);
            }

            // 固定50ms延迟，减少CPU使用
            thread::sleep(Duration::from_millis(50));
        }
    }
}

impl std::io::Seek for MinimalReader {
    fn seek(&mut self, _: std::io::SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Seek not supported"
        ))
    }
}

impl symphonia::core::io::MediaSource for MinimalReader {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

/// 极简的音频源
struct MinimalAudioSource {
    decoder: Box<dyn Decoder>,
    format: Box<dyn symphonia::core::formats::FormatReader>,
    sample_rate: u32,
    channels: u16,
    current_samples: Vec<f32>,
    sample_index: usize,
    packet_errors: u32,
}

impl MinimalAudioSource {
    fn new(
        decoder: Box<dyn Decoder>,
        format: Box<dyn symphonia::core::formats::FormatReader>,
        sample_rate: u32,
        channels: u16,
    ) -> Self {
        MinimalAudioSource {
            decoder,
            format,
            sample_rate,
            channels,
            current_samples: Vec::new(),
            sample_index: 0,
            packet_errors: 0,
        }
    }

    fn load_next_packet(&mut self) -> bool {
        // 尝试读取下一个packet
        match self.format.next_packet() {
            Ok(packet) => {
                match self.decoder.decode(&packet) {
                    Ok(decoded) => {
                        let spec = decoded.spec();
                        let duration = decoded.capacity() as u64;
                        let mut sample_buffer = SampleBuffer::<f32>::new(duration, *spec);
                        sample_buffer.copy_interleaved_ref(decoded);
                        self.current_samples = sample_buffer.samples().to_vec();
                        self.sample_index = 0;
                        self.packet_errors = 0; // Reset error count on success
                        true
                    }
                    Err(e) => {
                        self.packet_errors += 1;
                        if self.packet_errors < 5 {
                            // 尝试继续
                            true
                        } else {
                            println!("❌ Too many decode errors: {}", e);
                            false
                        }
                    }
                }
            }
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // 正常结束
                false
            }
            Err(_) => {
                // 其他错误，尝试继续
                self.packet_errors += 1;
                self.packet_errors < 5
            }
        }
    }
}

impl Iterator for MinimalAudioSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        // 如果当前samples用完了，加载下一个packet
        if self.sample_index >= self.current_samples.len() {
            if !self.load_next_packet() {
                return None;
            }
        }

        if self.sample_index < self.current_samples.len() {
            let sample = self.current_samples[self.sample_index];
            self.sample_index += 1;
            Some(sample)
        } else {
            None
        }
    }
}

impl Source for MinimalAudioSource {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.current_samples.len() - self.sample_index)
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[tokio::main]
async fn main() {
    println!("DAB.rs Minimal Streaming Test");
    println!("==============================\n");

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

    test_minimal_streaming(&stream_url).await;
}

async fn test_minimal_streaming(url: &str) {
    println!("🎵 Minimal Streaming Test");
    println!("Using extremely simple buffer and retry logic\n");

    // 创建极简缓冲区
    let buffer = MinimalBuffer::new();
    let buffer_clone = Arc::clone(&buffer);

    // 启动下载任务
    let url_clone = url.to_string();
    let download_handle = tokio::spawn(async move {
        download_minimal(&url_clone, buffer_clone).await;
    });

    // 等待更多初始数据 (5MB) - 更保守的缓冲
    println!("⏳ Buffering 5MB before starting (this may take a while)...");
    let start_time = Instant::now();

    loop {
        thread::sleep(Duration::from_millis(500)); // 每500ms检查一次

        let buf = buffer.lock().unwrap();
        let mb_available = buf.data.len() as f32 / (1024.0 * 1024.0);

        if mb_available >= 5.0 {
            println!("✅ Buffered {:.1} MB in {:.1}s",
                     mb_available,
                     start_time.elapsed().as_secs_f32());
            drop(buf);
            break;
        }

        // 打印进度
        if mb_available > 0.0 {
            print!("\r⏳ Buffered {:.1} MB / 5.0 MB", mb_available);
            use std::io::{self};
            io::stdout().flush().unwrap();
        }

        drop(buf);
    }

    println!("\n");

    // 创建极简Reader
    let reader = MinimalReader::new(buffer.clone());
    let mss = MediaSourceStream::new(Box::new(reader), Default::default());

    // 设置hint
    let hint = Hint::new();
    let fmt_opts: FormatOptions = Default::default();
    let meta_opts: MetadataOptions = Default::default();

    // Probe格式
    let probed = match symphonia::default::get_probe().format(&hint, mss, &fmt_opts, &meta_opts) {
        Ok(p) => p,
        Err(e) => {
            println!("❌ Failed to probe format: {}", e);
            download_handle.abort();
            return;
        }
    };

    let mut format = probed.format;
    let track = format.tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .expect("No supported audio tracks");

    let decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .expect("Failed to create decoder");

    let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let channels = track.codec_params.channels
        .map(|ch| ch.count() as u16)
        .unwrap_or(2);

    println!("✅ Decoder created: {}Hz, {} channels", sample_rate, channels);

    // 创建音频输出
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let sink = Sink::try_new(&stream_handle).unwrap();

    // 设置更大的音频缓冲
    sink.set_volume(0.8);

    // 创建音频源并播放
    let audio_source = MinimalAudioSource::new(decoder, format, sample_rate, channels);
    sink.append(audio_source);

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🎵 PLAYING (Minimal streaming)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("This version:");
    println!("  • 5MB pre-buffer (more conservative)");
    println!("  • 50ms fixed delays (no aggressive retries)");
    println!("  • Simple buffer without complex logic");
    println!("\nListen for pops/clicks!\n");

    // 播放监控
    for i in 0..60 {  // 播放60秒
        if sink.empty() {
            println!("\nPlayback finished");
            break;
        }

        let buf = buffer.lock().unwrap();
        let mb_buffered = buf.data.len() as f32 / (1024.0 * 1024.0);
        let mb_consumed = (buf.read_pos as f32) / (1024.0 * 1024.0);
        let mb_available = mb_buffered - mb_consumed;
        let complete = buf.complete;
        drop(buf);

        print!("\r⏱️  {} sec | Total: {:.1} MB | Available: {:.1} MB {}",
               i + 1, mb_buffered, mb_available,
               if complete { "| ✅ Downloaded" } else { "| ⏬ Downloading..." });

        use std::io::{self};
        io::stdout().flush().unwrap();
        thread::sleep(Duration::from_secs(1));
    }

    println!("\n\n✅ Test completed");
    sink.stop();
    download_handle.abort();
}

async fn download_minimal(url: &str, buffer: Arc<Mutex<MinimalBuffer>>) {
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
    println!("📥 Downloading {} MB", content_length / (1024 * 1024));

    let mut stream = response.bytes_stream();
    let mut downloaded = 0u64;
    let mut last_print = 0u64;

    while let Some(chunk_result) = stream.next().await {
        if let Ok(chunk) = chunk_result {
            // 写入缓冲区
            if let Ok(mut buf) = buffer.lock() {
                buf.data.extend_from_slice(&chunk);
            }

            downloaded += chunk.len() as u64;

            // 每5MB打印一次进度
            if downloaded - last_print >= 5 * 1024 * 1024 {
                println!("📥 Downloaded {} MB / {} MB",
                         downloaded / (1024 * 1024),
                         content_length / (1024 * 1024));
                last_print = downloaded;
            }
        }
    }

    // 标记完成
    if let Ok(mut buf) = buffer.lock() {
        buf.complete = true;
    }

    println!("✅ Download complete ({} MB)", downloaded / (1024 * 1024));
}

// Stream URL extraction (使用test_squid_direct的逻辑)
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
        serde_json::Value::Array(entries) => {
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

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(format!("API returned status: {}", response.status()).into());
    }

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