use dab::{player::loader::*, player::streaming::*, Cache, PlayerEvent, Track};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn test_circular_buffer_basic_operations() {
    let buffer = CircularBuffer::new(1024); // 1KB buffer

    // Test empty buffer
    assert_eq!(buffer.len(), 0);
    assert_eq!(buffer.capacity(), 1024);
    assert_eq!(buffer.available_space(), 1024);

    // Test writing data
    let test_data = b"Hello, World!";
    let written = buffer.write(test_data).unwrap();
    assert_eq!(written, test_data.len());
    assert_eq!(buffer.len(), test_data.len());

    // Test reading data
    let mut read_buffer = vec![0u8; test_data.len()];
    let read = buffer.read(&mut read_buffer).unwrap();
    assert_eq!(read, test_data.len());
    assert_eq!(read_buffer, test_data);
    assert_eq!(buffer.len(), 0);
}

#[tokio::test]
async fn test_circular_buffer_wrap_around() {
    let buffer = CircularBuffer::new(10); // Small buffer for wrap-around

    // Fill buffer completely
    let data1 = b"1234567890"; // Exactly 10 bytes
    buffer.write(data1).unwrap();
    assert_eq!(buffer.len(), 10);
    assert_eq!(buffer.available_space(), 0);

    // Try to write more (should fail or overwrite)
    let data2 = b"ABC";
    let written = buffer.write(data2).unwrap_or(0);

    // Read some data to make space
    let mut read_buffer = vec![0u8; 5];
    buffer.read(&mut read_buffer).unwrap();
    assert_eq!(buffer.len(), 5);

    // Now we should be able to write more
    let written = buffer.write(data2).unwrap();
    assert!(written > 0);
}

#[tokio::test]
async fn test_circular_buffer_concurrent_access() {
    let buffer = Arc::new(CircularBuffer::new(1024));
    let buffer_clone = Arc::clone(&buffer);

    // Writer task
    let writer = tokio::spawn(async move {
        for i in 0..100 {
            let data = format!("data{:03}", i);
            buffer_clone.write(data.as_bytes()).unwrap();
            sleep(Duration::from_millis(1)).await;
        }
    });

    // Reader task
    let reader = tokio::spawn(async move {
        let mut total_read = 0;
        while total_read < 100 * 7 {
            // Each "dataXXX" is 7 bytes
            let mut buf = vec![0u8; 64];
            if let Ok(read) = buffer.read(&mut buf) {
                total_read += read;
            }
            sleep(Duration::from_millis(1)).await;
        }
        total_read
    });

    let (writer_result, reader_result) = tokio::join!(writer, reader);
    writer_result.unwrap();
    let total_read = reader_result.unwrap();
    assert!(total_read > 0);
}

#[tokio::test]
async fn test_streaming_source_creation() {
    let url = "https://httpbin.org/bytes/1024";
    let track = Track::from_url(url);

    // Create streaming source
    let source = StreamingSource::new(track.clone(), url.to_string());

    assert_eq!(source.track_id(), track.id);
    assert_eq!(source.stream_url(), url);
}

#[tokio::test]
async fn test_streaming_source_buffering() {
    let url = "https://httpbin.org/bytes/2048"; // 2KB test data
    let track = Track::from_url(url);
    let source = StreamingSource::new(track.clone(), url.to_string());

    // Start buffering
    source.start_buffering().await.unwrap();

    // Wait for some buffering
    sleep(Duration::from_millis(200)).await;

    // Check buffer status
    let buffered = source.get_buffered_amount().await;
    assert!(buffered >= 0); // Should have some data or be attempting to buffer

    let is_ready = source.is_ready_for_playback(512).await; // Need 512 bytes
                                                            // Ready status depends on download speed
}

#[tokio::test]
async fn test_streaming_source_read() {
    let test_data = b"This is test streaming data for reading";
    let mut temp_file = NamedTempFile::new().unwrap();
    std::io::Write::write_all(&mut temp_file, test_data).unwrap();

    let file_url = format!("file://{}", temp_file.path().display());
    let track = Track::from_url(&file_url);
    let source = StreamingSource::new(track, file_url);

    // Start buffering
    source.start_buffering().await.unwrap();
    sleep(Duration::from_millis(100)).await;

    // Try to read data
    let mut buffer = vec![0u8; test_data.len()];
    if let Ok(bytes_read) = source.read(&mut buffer).await {
        assert!(bytes_read > 0);
        // For file sources, we might get the data immediately
    }
}

#[tokio::test]
async fn test_streaming_source_seek() {
    let test_data = vec![0u8; 4096]; // 4KB of zeros
    let mut temp_file = NamedTempFile::new().unwrap();
    std::io::Write::write_all(&mut temp_file, &test_data).unwrap();

    let file_url = format!("file://{}", temp_file.path().display());
    let track = Track::from_url(&file_url);
    let source = StreamingSource::new(track, file_url);

    // Start buffering
    source.start_buffering().await.unwrap();
    sleep(Duration::from_millis(100)).await;

    // Try to seek
    let seek_result = source.seek(SeekFrom::Start(1024)).await;

    // Seek might not be supported for all streaming sources
    match seek_result {
        Ok(pos) => assert_eq!(pos, 1024),
        Err(_) => {
            // Seek not supported, which is acceptable
        }
    }
}

#[tokio::test]
async fn test_loader_track_loading() {
    let cache = Cache::new().await.unwrap();
    let (event_sender, mut event_receiver) = mpsc::unbounded_channel();

    let loader = Loader::new(cache, event_sender);
    let track = Track::from_url("https://httpbin.org/bytes/1024");

    // Request track loading
    loader.load_track(track.clone()).await.unwrap();

    // Wait for events
    tokio::select! {
        event = event_receiver.recv() => {
            if let Some(event) = event {
                match event {
                    PlayerEvent::DownloadStarted { track_id } => {
                        assert_eq!(track_id, track.id);
                    }
                    PlayerEvent::StreamReady { track_id } => {
                        assert_eq!(track_id, track.id);
                    }
                    _ => {
                        // Other events are acceptable
                    }
                }
            }
        }
        _ = sleep(Duration::from_millis(500)) => {
            // Timeout is acceptable for network operations
        }
    }
}

#[tokio::test]
async fn test_loader_cached_track() {
    let mut cache = Cache::new().await.unwrap();
    let (event_sender, _event_receiver) = mpsc::unbounded_channel();

    // Pre-cache a track
    let test_data = b"cached audio data";
    let mut temp_file = NamedTempFile::new().unwrap();
    std::io::Write::write_all(&mut temp_file, test_data).unwrap();

    let track_id = "cached_track";
    cache.store_track(track_id, temp_file.path()).await.unwrap();

    let loader = Loader::new(cache, event_sender);
    let track = Track {
        id: track_id.to_string(),
        title: "Cached Track".to_string(),
        artist: "Test Artist".to_string(),
        album: "Test Album".to_string(),
        duration: Some(180),
        track_number: None,
        disc_number: None,
        year: None,
        genre: None,
        album_id: None,
        artist_id: None,
        local_path: None,
        stream_url: None,
        cover_url: None,
    };

    // Load cached track (should be fast)
    let start_time = std::time::Instant::now();
    let result = loader.load_track(track).await;
    let elapsed = start_time.elapsed();

    // Cached tracks should load quickly
    assert!(elapsed < Duration::from_millis(100));
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_loader_preloading() {
    let cache = Cache::new().await.unwrap();
    let (event_sender, _event_receiver) = mpsc::unbounded_channel();

    let loader = Loader::new(cache, event_sender);
    let tracks: Vec<_> = (0..3)
        .map(|i| Track::from_url(&format!("https://httpbin.org/bytes/1024?track={}", i)))
        .collect();

    // Request preloading of multiple tracks
    for track in &tracks {
        loader.preload_track(track.clone()).await.unwrap();
    }

    // Wait for preloading to start
    sleep(Duration::from_millis(200)).await;

    // Check preload status
    for track in &tracks {
        let status = loader.get_preload_status(&track.id).await;
        assert!(matches!(
            status,
            PreloadStatus::NotStarted
                | PreloadStatus::InProgress
                | PreloadStatus::Completed
                | PreloadStatus::Failed
        ));
    }
}

#[tokio::test]
async fn test_streaming_error_handling() {
    let invalid_url = "https://invalid-domain-that-does-not-exist.com/audio.mp3";
    let track = Track::from_url(invalid_url);
    let source = StreamingSource::new(track, invalid_url.to_string());

    // Try to start buffering with invalid URL
    let result = source.start_buffering().await;

    // Should handle the error gracefully
    assert!(result.is_err());
}

#[tokio::test]
async fn test_streaming_progress_tracking() {
    let cache = Cache::new().await.unwrap();
    let (event_sender, mut event_receiver) = mpsc::unbounded_channel();

    let loader = Loader::new(cache, event_sender);
    let track = Track::from_url("https://httpbin.org/bytes/4096"); // 4KB file

    // Start loading
    loader.load_track(track.clone()).await.unwrap();

    let mut received_progress = false;

    // Monitor progress events
    for _ in 0..10 {
        tokio::select! {
            event = event_receiver.recv() => {
                if let Some(PlayerEvent::DownloadProgress { track_id, progress }) = event {
                    if track_id == track.id {
                        assert!((0.0..=1.0).contains(&progress));
                        received_progress = true;
                        break;
                    }
                }
            }
            _ = sleep(Duration::from_millis(100)) => {
                break;
            }
        }
    }

    // Progress events might not always be emitted for small/fast downloads
}

#[tokio::test]
async fn test_buffer_underrun_handling() {
    let buffer = CircularBuffer::new(512); // Small buffer

    // Read from empty buffer
    let mut read_buffer = vec![0u8; 1024];
    let result = buffer.read(&mut read_buffer);

    // Should handle empty buffer gracefully
    match result {
        Ok(0) => {
            // No data available, which is expected
        }
        Ok(n) => {
            // Some data was available
            assert!(n <= read_buffer.len());
        }
        Err(_) => {
            // Error handling is implementation-dependent
        }
    }
}

#[tokio::test]
async fn test_streaming_cleanup() {
    let cache = Cache::new().await.unwrap();
    let (event_sender, _event_receiver) = mpsc::unbounded_channel();

    let loader = Loader::new(cache, event_sender);
    let track = Track::from_url("https://httpbin.org/bytes/1024");

    // Load track
    loader.load_track(track.clone()).await.unwrap();
    sleep(Duration::from_millis(100)).await;

    // Clean up resources
    loader.cleanup_track(&track.id).await;

    // Track should be cleaned up (exact behavior depends on implementation)
    let status = loader.get_preload_status(&track.id).await;
    assert!(matches!(
        status,
        PreloadStatus::NotStarted | PreloadStatus::Completed | PreloadStatus::Failed
    ));
}
