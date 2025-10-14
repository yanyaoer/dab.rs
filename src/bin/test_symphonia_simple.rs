use futures_util::StreamExt;
use reqwest;
use rodio::{OutputStream, Sink, Source};
use serde_json;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

// 简化的流式缓冲区 - 使用双缓冲策略
struct SimpleStreamBuffer {
    // 使用两个缓冲区交替使用
    buffer_a: Vec<u8>,
    buffer_b: Vec<u8>,
    active_buffer: bool, // true = A, false = B

    write_pos: usize,
    read_pos: usize,
    total_written: usize,
    total_read: usize,
    complete: bool,

    // 统计
    underrun_count: u32,
    last_underrun: Option<Instant>,
}

impl SimpleStreamBuffer {
    fn new(capacity: usize) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(SimpleStreamBuffer {
            buffer_a: Vec::with_capacity(capacity),
            buffer_b: Vec::with_capacity(capacity),
            active_buffer: true,
            write_pos: 0,
            read_pos: 0,
            total_written: 0,
            total_read: 0,
            complete: false,
            underrun_count: 0,
            last_underrun: None,
        }))
    }

    fn write_data(&mut self, chunk: &[u8]) {
        // 写入当前活动缓冲区
        let buffer = if self.active_buffer {
            &mut self.buffer_a
        } else {
            &mut self.buffer_b
        };

        buffer.extend_from_slice(chunk);
        self.total_written += chunk.len();
    }

    fn bytes_available(&self) -> usize {
        self.total_written.saturating_sub(self.total_read)
    }

    fn read_bytes(&mut self, buf: &mut [u8]) -> usize {
        let available = self.bytes_available();
        if available == 0 {
            return 0;
        }

        let to_read = buf.len().min(available);

        // 简化的读取逻辑 - 从合并的数据中读取
        let all_data = if self.active_buffer {
            &self.buffer_a
        } else {
            &self.buffer_b
        };

        if self.read_pos + to_read <= all_data.len() {
            buf[..to_read].copy_from_slice(&all_data[self.read_pos..self.read_pos + to_read]);
            self.read_pos += to_read;
            self.total_read += to_read;
            to_read
        } else {
            // 需要从两个缓冲区读取
            let first_part = all_data.len() - self.read_pos;
            if first_part > 0 {
                buf[..first_part].copy_from_slice(&all_data[self.read_pos..]);
            }

            // 切换缓冲区并继续读取
            self.active_buffer = !self.active_buffer;
            self.read_pos = 0;

            let other_buffer = if self.active_buffer {
                &self.buffer_a
            } else {
                &self.buffer_b
            };

            let second_part = (to_read - first_part).min(other_buffer.len());
            if second_part > 0 {
                buf[first_part..first_part + second_part].copy_from_slice(&other_buffer[..second_part]);
                self.read_pos = second_part;
            }

            self.total_read += first_part + second_part;
            first_part + second_part
        }
    }
}

// 简化的读取器
struct SimplifiedReader {
    buffer: Arc<Mutex<SimpleStreamBuffer>>,
    silence_buffer: Vec<u8>,
}

impl SimplifiedReader {
    fn new(buffer: Arc<Mutex<SimpleStreamBuffer>>) -> Self {
        // 预生成静音数据 (10ms worth at 48kHz stereo)
        let silence_samples = 48000 * 2 * 10 / 1000; // samples * channels * ms / 1000
        SimplifiedReader {
            buffer,
            silence_buffer: vec![0u8; silence_samples * 2], // 16-bit samples
        }
    }
}

impl Read for SimplifiedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // 首先尝试直接读取
        {
            let mut buffer = self.buffer.lock().unwrap();
            let read = buffer.read_bytes(buf);
            if read > 0 {
                return Ok(read);
            }

            // 如果已完成且没有数据，返回EOF
            if buffer.complete {
                return Ok(0);
            }
        }

        // 没有数据可读，等待数据
        let wait_start = Instant::now();
        let max_wait = Duration::from_millis(100); // 最多等100ms

        loop {
            // 等待一段时间让下载线程写入数据
            thread::sleep(Duration::from_millis(10)); // 更长的等待时间，减少CPU使用

            let mut buffer = self.buffer.lock().unwrap();

            // 再次尝试读取
            let read = buffer.read_bytes(buf);
            if read > 0 {
                return Ok(read);
            }

            // 检查是否完成
            if buffer.complete {
                return Ok(0);
            }

            // 检查是否超时
            if wait_start.elapsed() > max_wait {
                // 记录underrun
                buffer.underrun_count += 1;
                let now = Instant::now();

                // 只在距离上次underrun超过100ms时打印
                let should_print = buffer.last_underrun
                    .map(|last| now.duration_since(last) > Duration::from_millis(100))
                    .unwrap_or(true);

                if should_print {
                    println!("⚠️  Buffer underrun #{} - inserting silence", buffer.underrun_count);
                    buffer.last_underrun = Some(now);
                }

                // 返回静音数据而不是错误
                let silence_len = buf.len().min(self.silence_buffer.len());
                buf[..silence_len].copy_from_slice(&self.silence_buffer[..silence_len]);
                return Ok(silence_len);
            }
        }
    }
}

impl Seek for SimplifiedReader {
    fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
        // 流式源不支持seek
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Streaming source does not support seeking",
        ))
    }
}

impl symphonia::core::io::MediaSource for SimplifiedReader {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

// 简化的音频源
struct SimplifiedAudioSource {
    decoder: Box<dyn Decoder>,
    format: Box<dyn symphonia::core::formats::FormatReader>,
    sample_rate: u32,
    channels: u16,
    current_samples: Vec<f32>,
    sample_index: usize,
}

impl SimplifiedAudioSource {
    fn new(
        decoder: Box<dyn Decoder>,
        format: Box<dyn symphonia::core::formats::FormatReader>,
        sample_rate: u32,
        channels: u16,
    ) -> Self {
        SimplifiedAudioSource {
            decoder,
            format,
            sample_rate,
            channels,
            current_samples: Vec::new(),
            sample_index: 0,
        }
    }

    fn load_next_packet(&mut self) -> bool {
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
                        true
                    }
                    Err(_) => false,
                }
            }
            Err(_) => false,
        }
    }
}

impl Iterator for SimplifiedAudioSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
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

impl Source for SimplifiedAudioSource {
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
    println!("DAB.rs Simplified Symphonia Streaming Test");
    println!("===========================================\n");

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

    println!("Testing simplified streaming with better underrun handling");
    println!("This version:");
    println!("  • Uses simpler double-buffer approach");
    println!("  • Inserts silence on underruns instead of errors");
    println!("  • Has smoother retry logic with longer waits");
    println!("");

    test_simplified_streaming(&stream_url).await;
}

async fn test_simplified_streaming(url: &str) {
    println!("🎵 Simplified Streaming Test\n");

    // 创建缓冲区 (20MB容量)
    let buffer = SimpleStreamBuffer::new(20 * 1024 * 1024);
    let buffer_clone = Arc::clone(&buffer);

    // 启动下载
    let url_clone = url.to_string();
    let download_handle = tokio::spawn(async move {
        download_to_buffer(&url_clone, buffer_clone).await;
    });

    // 等待更多初始数据 (2MB)
    println!("⏳ Buffering 2MB before starting playback...");
    let start_time = Instant::now();

    loop {
        let buf = buffer.lock().unwrap();
        let mb_available = buf.bytes_available() as f32 / (1024.0 * 1024.0);
        if mb_available >= 2.0 {
            println!("✅ Buffered {:.1} MB in {:.1}s",
                     mb_available,
                     start_time.elapsed().as_secs_f32());
            drop(buf);
            break;
        }
        drop(buf);
        thread::sleep(Duration::from_millis(100));
    }

    // 创建简化的读取器
    let reader = SimplifiedReader::new(buffer.clone());
    let mss = MediaSourceStream::new(Box::new(reader), Default::default());
    let hint = Hint::new();
    let fmt_opts: FormatOptions = Default::default();
    let meta_opts: MetadataOptions = Default::default();

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

    let audio_source = SimplifiedAudioSource::new(decoder, format, sample_rate, channels);
    sink.set_volume(0.8);
    sink.append(audio_source);

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🎵 PLAYING (Simplified streaming)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Listen for pops/clicks!\n");

    // 监控播放
    for i in 0..30 {
        if sink.empty() {
            println!("\nPlayback finished");
            break;
        }

        let buf = buffer.lock().unwrap();
        let mb_buffered = buf.bytes_available() as f32 / (1024.0 * 1024.0);
        let underruns = buf.underrun_count;
        let complete = buf.complete;
        drop(buf);

        print!("\r⏱️  {} sec | Buffer: {:.1} MB | Underruns: {} {}",
               i + 1, mb_buffered, underruns,
               if complete { "| ✅ Downloaded" } else { "" });

        use std::io::{self, Write};
        io::stdout().flush().unwrap();
        thread::sleep(Duration::from_secs(1));
    }

    println!("\n\n✅ Test completed");
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
    println!("📥 Downloading {} MB", content_length / (1024 * 1024));

    let mut stream = response.bytes_stream();
    let mut downloaded = 0u64;

    while let Some(chunk_result) = stream.next().await {
        if let Ok(chunk) = chunk_result {
            let mut buf = buffer.lock().unwrap();
            buf.write_data(&chunk);
            drop(buf);

            downloaded += chunk.len() as u64;

            // Print download progress less frequently
            if downloaded % (5 * 1024 * 1024) < chunk.len() as u64 {
                println!("📥 Downloaded {} MB", downloaded / (1024 * 1024));
            }
        }
    }

    let mut buf = buffer.lock().unwrap();
    buf.complete = true;
    println!("✅ Download complete");
}

// Stream URL extraction (from test_squid_direct)
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