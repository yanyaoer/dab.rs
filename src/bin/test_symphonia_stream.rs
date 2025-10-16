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

// 流式缓冲区
struct StreamBuffer {
    data: Vec<u8>,
    write_pos: usize,
    read_pos: usize,
    complete: bool,
    underrun_count: u32,
}

impl StreamBuffer {
    fn new(capacity: usize) -> Self {
        StreamBuffer {
            data: Vec::with_capacity(capacity),
            write_pos: 0,
            read_pos: 0,
            complete: false,
            underrun_count: 0,
        }
    }

    fn write_data(&mut self, chunk: &[u8]) {
        self.data.extend_from_slice(chunk);
        self.write_pos = self.data.len();
    }

    fn bytes_available(&self) -> usize {
        self.write_pos.saturating_sub(self.read_pos)
    }
}

// StreamBuffer的包装器
struct StreamBufferSource {
    buffer: Arc<Mutex<StreamBuffer>>,
}

impl StreamBufferSource {
    fn new(buffer: Arc<Mutex<StreamBuffer>>) -> Self {
        StreamBufferSource { buffer }
    }
}

impl Read for StreamBufferSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut retries = 0;
        let max_retries = 500; // 5秒超时

        loop {
            let mut buffer = self.buffer.lock().unwrap();
            let available = buffer.bytes_available();

            if available > 0 {
                let to_read = buf.len().min(available);
                buf[..to_read]
                    .copy_from_slice(&buffer.data[buffer.read_pos..buffer.read_pos + to_read]);
                buffer.read_pos += to_read;
                return Ok(to_read);
            }

            if buffer.complete && available == 0 {
                return Ok(0); // EOF
            }

            // 记录欠载
            if retries == 0 && !buffer.complete {
                buffer.underrun_count += 1;
                if buffer.underrun_count % 10 == 1 {
                    println!("⚠️  Buffer underrun #{}", buffer.underrun_count);
                }
            }

            drop(buffer);

            retries += 1;
            if retries > max_retries {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "Timeout waiting for data",
                ));
            }

            // 更激进的重试策略（类似DAB修复前的行为）
            if retries < 3 {
                thread::sleep(Duration::from_millis(1)); // 前3次只等1ms
            } else if retries < 10 {
                thread::sleep(Duration::from_millis(5)); // 接下来等5ms
            } else {
                thread::sleep(Duration::from_millis(10)); // 最后等10ms
            }
        }
    }
}

impl Seek for StreamBufferSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match pos {
            SeekFrom::Start(offset) => {
                let mut buffer = self.buffer.lock().unwrap();
                buffer.read_pos = offset as usize;
                Ok(offset)
            }
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Only SeekFrom::Start is supported",
            )),
        }
    }
}

impl symphonia::core::io::MediaSource for StreamBufferSource {
    fn is_seekable(&self) -> bool {
        false // 流式源不可seek
    }

    fn byte_len(&self) -> Option<u64> {
        None // 长度未知
    }
}

// 音频源适配器
struct DecodedAudioSource {
    decoder: Arc<Mutex<Box<dyn Decoder>>>,
    format: Arc<Mutex<Box<dyn symphonia::core::formats::FormatReader>>>,
    sample_rate: u32,
    channels: u16,
    current_samples: Vec<f32>,
    sample_index: usize,
    finished: bool,
}

impl DecodedAudioSource {
    fn new(
        decoder: Box<dyn Decoder>,
        format: Box<dyn symphonia::core::formats::FormatReader>,
        sample_rate: u32,
        channels: u16,
    ) -> Self {
        DecodedAudioSource {
            decoder: Arc::new(Mutex::new(decoder)),
            format: Arc::new(Mutex::new(format)),
            sample_rate,
            channels,
            current_samples: Vec::new(),
            sample_index: 0,
            finished: false,
        }
    }

    fn load_next_packet(&mut self) -> bool {
        if self.finished {
            return false;
        }

        let mut format = self.format.lock().unwrap();
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                self.finished = true;
                return false;
            }
            Err(e) => {
                println!("❌ Packet read error: {}", e);
                self.finished = true;
                return false;
            }
        };

        drop(format);

        let mut decoder = self.decoder.lock().unwrap();
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = decoded.spec();
                let duration = decoded.capacity() as u64;
                let mut sample_buffer = SampleBuffer::<f32>::new(duration, *spec);
                sample_buffer.copy_interleaved_ref(decoded);
                self.current_samples = sample_buffer.samples().to_vec();
                self.sample_index = 0;
                true
            }
            Err(e) => {
                println!("❌ Decode error: {}", e);
                false
            }
        }
    }
}

impl Iterator for DecodedAudioSource {
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

impl Source for DecodedAudioSource {
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
        None // 流式源的时长未知
    }
}

#[tokio::main]
async fn main() {
    println!("DAB.rs Symphonia Streaming Test");
    println!("================================\n");

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

    println!("Select buffer size:");
    println!("1. Large buffer (1MB) - Should be stable");
    println!("2. Medium buffer (500KB)");
    println!("3. Small buffer (100KB) - May have underruns");
    println!("4. Tiny buffer (50KB) - Stress test");
    println!("Enter choice (1/2/3/4): ");

    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();

    let min_buffer_kb = match input.trim() {
        "1" => 1024,
        "2" => 500,
        "3" => 100,
        "4" => 50,
        _ => {
            println!("Invalid choice");
            return;
        }
    };

    test_symphonia_streaming(&stream_url, min_buffer_kb).await;
}

async fn test_symphonia_streaming(url: &str, min_buffer_kb: usize) {
    println!("\n🎵 Symphonia Streaming Test");
    println!("Initial buffer: {} KB\n", min_buffer_kb);

    let buffer = Arc::new(Mutex::new(StreamBuffer::new(50 * 1024 * 1024)));
    let buffer_clone = Arc::clone(&buffer);

    // 启动下载
    let url_clone = url.to_string();
    let download_handle = tokio::spawn(async move {
        download_to_buffer(&url_clone, buffer_clone).await;
    });

    // 等待初始缓冲
    println!("⏳ Buffering {} KB...", min_buffer_kb);
    let start_time = Instant::now();

    loop {
        let buf = buffer.lock().unwrap();
        let kb_available = buf.bytes_available() / 1024;
        if kb_available >= min_buffer_kb {
            println!(
                "✅ Buffered {} KB in {:.1}s",
                kb_available,
                start_time.elapsed().as_secs_f32()
            );
            drop(buf);
            break;
        }
        drop(buf);
        thread::sleep(Duration::from_millis(100));
    }

    // 创建symphonia解码器
    let source = StreamBufferSource::new(buffer.clone());
    let mss = MediaSourceStream::new(Box::new(source), Default::default());
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

    let audio_source = DecodedAudioSource::new(decoder, format, sample_rate, channels);
    sink.set_volume(0.8);
    sink.append(audio_source);

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🎵 PLAYING (True streaming with Symphonia)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Monitor for pops/clicks and underruns!\n");

    // 监控播放
    for i in 0..30 {
        if sink.empty() {
            println!("\nPlayback finished");
            break;
        }

        let buf = buffer.lock().unwrap();
        let kb_buffered = buf.bytes_available() / 1024;
        let underruns = buf.underrun_count;
        let complete = buf.complete;
        drop(buf);

        print!(
            "\r⏱️  {} sec | Buffer: {} KB | Underruns: {} {}",
            i + 1,
            kb_buffered,
            underruns,
            if complete { "| ✅ Downloaded" } else { "" }
        );

        use std::io::{self, Write};
        io::stdout().flush().unwrap();
        thread::sleep(Duration::from_secs(1));
    }

    println!("\n\n✅ Test completed");
    sink.stop();
    download_handle.abort();
}

async fn download_to_buffer(url: &str, buffer: Arc<Mutex<StreamBuffer>>) {
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
            if downloaded % (5 * 1024 * 1024) == 0 {
                println!("📥 Downloaded {} MB", downloaded / (1024 * 1024));
            }
        }
    }

    let mut buf = buffer.lock().unwrap();
    buf.complete = true;
    println!("✅ Download complete");
}

// Stream URL获取函数 (从test_squid_direct复制)
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
        Err(format!(
            "Could not extract stream URL from response: {}",
            if body.len() > 200 {
                &body[..200]
            } else {
                &body
            }
        )
        .into())
    }
}
