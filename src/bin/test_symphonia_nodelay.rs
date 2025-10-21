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
            data: Vec::with_capacity(20 * 1024 * 1024), // 20MB capacity
            read_pos: 0,
            complete: false,
        }))
    }
}

/// 无延迟的Reader - 立即返回或填充零
struct NoDelayReader {
    buffer: Arc<Mutex<MinimalBuffer>>,
    zero_buffer: Vec<u8>,
    silence_inserted: usize,
}

impl NoDelayReader {
    fn new(buffer: Arc<Mutex<MinimalBuffer>>) -> Self {
        NoDelayReader {
            buffer,
            zero_buffer: vec![0u8; 4096], // 4KB零缓冲
            silence_inserted: 0,
        }
    }
}

impl Read for NoDelayReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // 立即尝试获取锁，不等待
        match self.buffer.try_lock() {
            Ok(mut buffer) => {
                let available = buffer.data.len() - buffer.read_pos;

                if available > 0 {
                    // 有数据可读
                    let to_read = buf.len().min(available);
                    buf[..to_read]
                        .copy_from_slice(&buffer.data[buffer.read_pos..buffer.read_pos + to_read]);
                    buffer.read_pos += to_read;

                    // 重置静音计数
                    if self.silence_inserted > 0 {
                        println!(
                            "📊 Recovered after {} bytes of silence",
                            self.silence_inserted
                        );
                        self.silence_inserted = 0;
                    }

                    return Ok(to_read);
                }

                // 没有数据，检查是否完成
                if buffer.complete && available == 0 {
                    return Ok(0); // EOF
                }

                // 释放锁
                drop(buffer);
            }
            Err(_) => {
                // 无法获取锁，立即返回零数据
            }
        }

        // 立即返回零数据（静音），不等待
        let silence_len = buf.len().min(self.zero_buffer.len());
        buf[..silence_len].copy_from_slice(&self.zero_buffer[..silence_len]);

        self.silence_inserted += silence_len;
        if self.silence_inserted % 40960 == 0 {
            // 每40KB打印一次
            println!(
                "⚠️  Inserted {} KB of silence",
                self.silence_inserted / 1024
            );
        }

        Ok(silence_len)
    }
}

impl std::io::Seek for NoDelayReader {
    fn seek(&mut self, _: std::io::SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Seek not supported",
        ))
    }
}

impl symphonia::core::io::MediaSource for NoDelayReader {
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
    silence_frames: Vec<f32>,
    total_silence_inserted: usize,
}

impl MinimalAudioSource {
    fn new(
        decoder: Box<dyn Decoder>,
        format: Box<dyn symphonia::core::formats::FormatReader>,
        sample_rate: u32,
        channels: u16,
    ) -> Self {
        // 预生成10ms的静音
        let silence_samples = ((sample_rate as usize) * (channels as usize) * 10) / 1000;

        MinimalAudioSource {
            decoder,
            format,
            sample_rate,
            channels,
            current_samples: Vec::new(),
            sample_index: 0,
            silence_frames: vec![0.0; silence_samples],
            total_silence_inserted: 0,
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

                        if self.total_silence_inserted > 0 {
                            println!(
                                "🎵 Audio recovered after {} silence samples",
                                self.total_silence_inserted
                            );
                            self.total_silence_inserted = 0;
                        }

                        true
                    }
                    Err(_) => {
                        // 解码错误，插入静音
                        self.current_samples = self.silence_frames.clone();
                        self.sample_index = 0;
                        self.total_silence_inserted += self.silence_frames.len();
                        true
                    }
                }
            }
            Err(_) => {
                // 无数据，插入静音而不是停止
                if self.total_silence_inserted < self.sample_rate as usize * 10 {
                    // 最多10秒静音
                    self.current_samples = self.silence_frames.clone();
                    self.sample_index = 0;
                    self.total_silence_inserted += self.silence_frames.len();
                    true
                } else {
                    false // 10秒后停止
                }
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
    println!("DAB.rs NO-DELAY Streaming Test");
    println!("===============================\n");

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

    test_no_delay_streaming(&stream_url).await;
}

async fn test_no_delay_streaming(url: &str) {
    println!("🎵 No-Delay Streaming Test");
    println!("NO sleep delays - immediate response\n");

    // 创建缓冲区
    let buffer = MinimalBuffer::new();
    let buffer_clone = Arc::clone(&buffer);

    // 启动下载任务
    let url_clone = url.to_string();
    let download_handle = tokio::spawn(async move {
        download_fast(&url_clone, buffer_clone).await;
    });

    // 等待初始数据 (10MB) - 更大的初始缓冲
    println!("⏳ Buffering 10MB before starting...");
    let start_time = Instant::now();

    loop {
        thread::sleep(Duration::from_millis(100)); // 检查间隔

        let buf = buffer.lock().unwrap();
        let mb_available = buf.data.len() as f32 / (1024.0 * 1024.0);

        if mb_available >= 10.0 {
            println!(
                "✅ Buffered {:.1} MB in {:.1}s",
                mb_available,
                start_time.elapsed().as_secs_f32()
            );
            drop(buf);
            break;
        }

        // 打印进度
        if mb_available > 0.0 {
            print!("\r⏳ Buffered {:.1} MB / 10.0 MB", mb_available);
            use std::io::{self};
            io::stdout().flush().unwrap();
        }

        drop(buf);
    }

    println!("\n");

    // 创建无延迟Reader
    let reader = NoDelayReader::new(buffer.clone());
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

    let format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .expect("No supported audio tracks");

    let decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .expect("Failed to create decoder");

    let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let channels = track
        .codec_params
        .channels
        .map(|ch| ch.count() as u16)
        .unwrap_or(2);

    println!(
        "✅ Decoder created: {}Hz, {} channels",
        sample_rate, channels
    );

    // 创建音频输出
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let sink = Sink::try_new(&stream_handle).unwrap();

    // 设置音量
    sink.set_volume(0.8);

    // 创建音频源并播放
    let audio_source = MinimalAudioSource::new(decoder, format, sample_rate, channels);
    sink.append(audio_source);

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🎵 PLAYING (No-Delay streaming)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("This version:");
    println!("  • 10MB pre-buffer (very large)");
    println!("  • NO sleep delays - immediate response");
    println!("  • Inserts silence instead of waiting");
    println!("  • Should have NO pops (only silence gaps)");
    println!("\nListen for pops vs silence gaps!\n");

    // 播放监控
    for i in 0..60 {
        // 播放60秒
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

        print!(
            "\r⏱️  {} sec | Total: {:.1} MB | Available: {:.1} MB {}",
            i + 1,
            mb_buffered,
            mb_available,
            if complete {
                "| ✅ Downloaded"
            } else {
                "| ⏬ Downloading..."
            }
        );

        use std::io::{self};
        io::stdout().flush().unwrap();
        thread::sleep(Duration::from_secs(1));
    }

    println!("\n\n✅ Test completed");
    sink.stop();
    download_handle.abort();
}

async fn download_fast(url: &str, buffer: Arc<Mutex<MinimalBuffer>>) {
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
    println!(
        "📥 Downloading {} MB (FAST mode)",
        content_length / (1024 * 1024)
    );

    let mut stream = response.bytes_stream();
    let mut downloaded = 0u64;
    let mut last_print = 0u64;

    while let Some(chunk_result) = stream.next().await {
        if let Ok(chunk) = chunk_result {
            // 写入缓冲区 - 尝试立即获取锁
            loop {
                if let Ok(mut buf) = buffer.try_lock() {
                    buf.data.extend_from_slice(&chunk);
                    break;
                }
                // 如果无法获取锁，立即重试
                thread::yield_now();
            }

            downloaded += chunk.len() as u64;

            // 每5MB打印一次进度
            if downloaded - last_print >= 5 * 1024 * 1024 {
                println!(
                    "📥 Downloaded {} MB / {} MB (FAST)",
                    downloaded / (1024 * 1024),
                    content_length / (1024 * 1024)
                );
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
