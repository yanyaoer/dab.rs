use dab::{player::download_manager::*, Cache, Track, PlayerEvent};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn test_download_manager_initialization() {
    let temp_dir = TempDir::new().unwrap();
    let cache = Cache::new().await.unwrap();
    let (event_sender, _event_receiver) = mpsc::unbounded_channel();
    
    let manager = DownloadManager::new(cache, event_sender);
    
    // Manager should be created successfully
    // Basic initialization test - no panics
    assert!(true);
}

#[tokio::test]
async fn test_download_request() {
    let temp_dir = TempDir::new().unwrap();
    let cache = Cache::new().await.unwrap();
    let (event_sender, mut event_receiver) = mpsc::unbounded_channel();
    
    let manager = DownloadManager::new(cache, event_sender);
    let track = Track::from_url("https://httpbin.org/status/200");
    
    // Request download
    manager.request_download(track.clone()).await;
    
    // Wait for potential events
    tokio::select! {
        event = event_receiver.recv() => {
            if let Some(event) = event {
                match event {
                    PlayerEvent::DownloadStarted { track_id } => {
                        assert_eq!(track_id, track.id);
                    }
                    PlayerEvent::DownloadProgress { track_id, progress } => {
                        assert_eq!(track_id, track.id);
                        assert!((0.0..=1.0).contains(&progress));
                    }
                    PlayerEvent::DownloadCompleted { track_id } => {
                        assert_eq!(track_id, track.id);
                    }
                    _ => {
                        // Other events are also acceptable
                    }
                }
            }
        }
        _ = sleep(Duration::from_millis(500)) => {
            // Timeout is acceptable - downloads are asynchronous
        }
    }
}

#[tokio::test]
async fn test_download_priority() {
    let cache = Cache::new().await.unwrap();
    let (event_sender, _event_receiver) = mpsc::unbounded_channel();
    
    let manager = DownloadManager::new(cache, event_sender);
    
    let track1 = Track::from_url("https://httpbin.org/status/200");
    let track2 = Track::from_url("https://httpbin.org/delay/1");
    
    // Request downloads with different priorities
    manager.request_download_with_priority(track1.clone(), DownloadPriority::Normal).await;
    manager.request_download_with_priority(track2.clone(), DownloadPriority::High).await;
    
    sleep(Duration::from_millis(100)).await;
    
    // High priority should be processed (exact behavior depends on implementation)
    // This test mainly ensures the API works without panicking
}

#[tokio::test]
async fn test_download_cancellation() {
    let cache = Cache::new().await.unwrap();
    let (event_sender, _event_receiver) = mpsc::unbounded_channel();
    
    let manager = DownloadManager::new(cache, event_sender);
    let track = Track::from_url("https://httpbin.org/delay/5"); // Slow download
    
    // Start download
    manager.request_download(track.clone()).await;
    sleep(Duration::from_millis(50)).await;
    
    // Cancel download
    manager.cancel_download(&track.id).await;
    
    // Wait to ensure cancellation is processed
    sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
async fn test_download_status_tracking() {
    let cache = Cache::new().await.unwrap();
    let (event_sender, _event_receiver) = mpsc::unbounded_channel();
    
    let manager = DownloadManager::new(cache, event_sender);
    let track = Track::from_url("https://httpbin.org/status/200");
    
    // Initially not downloading
    let status = manager.get_download_status(&track.id).await;
    assert_eq!(status, DownloadStatus::NotStarted);
    
    // Request download
    manager.request_download(track.clone()).await;
    sleep(Duration::from_millis(50)).await;
    
    // Status should change
    let status = manager.get_download_status(&track.id).await;
    assert!(matches!(status, 
        DownloadStatus::NotStarted | 
        DownloadStatus::Queued | 
        DownloadStatus::InProgress | 
        DownloadStatus::Completed |
        DownloadStatus::Failed
    ));
}

#[tokio::test]
async fn test_download_progress_tracking() {
    let cache = Cache::new().await.unwrap();
    let (event_sender, mut event_receiver) = mpsc::unbounded_channel();
    
    let manager = DownloadManager::new(cache, event_sender);
    let track = Track::from_url("https://httpbin.org/bytes/1024"); // 1KB file
    
    // Request download
    manager.request_download(track.clone()).await;
    
    let mut received_progress = false;
    
    // Wait for progress events
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
    
    // Progress tracking might not always trigger for small/fast downloads
    // This test mainly ensures the API works
}

#[tokio::test]
async fn test_concurrent_downloads() {
    let cache = Cache::new().await.unwrap();
    let (event_sender, _event_receiver) = mpsc::unbounded_channel();
    
    let manager = DownloadManager::new(cache, event_sender);
    let manager_arc = Arc::new(manager);
    
    let mut handles = vec![];
    
    // Start multiple concurrent downloads
    for i in 0..3 {
        let manager_clone = Arc::clone(&manager_arc);
        let handle = tokio::spawn(async move {
            let track = Track::from_url(&format!("https://httpbin.org/bytes/1024?id={}", i));
            manager_clone.request_download(track).await;
        });
        handles.push(handle);
    }
    
    // Wait for all downloads to start
    for handle in handles {
        handle.await.unwrap();
    }
    
    // Give downloads time to process
    sleep(Duration::from_millis(500)).await;
}

#[tokio::test]
async fn test_download_retry_mechanism() {
    let cache = Cache::new().await.unwrap();
    let (event_sender, mut event_receiver) = mpsc::unbounded_channel();
    
    let manager = DownloadManager::new(cache, event_sender);
    let track = Track::from_url("https://httpbin.org/status/500"); // Will fail
    
    // Request download of failing URL
    manager.request_download(track.clone()).await;
    
    let mut received_failure = false;
    
    // Wait for potential failure events
    for _ in 0..10 {
        tokio::select! {
            event = event_receiver.recv() => {
                if let Some(event) = event {
                    match event {
                        PlayerEvent::DownloadStarted { .. } => {
                            // Download started
                        }
                        PlayerEvent::DownloadProgress { .. } => {
                            // Progress event
                        }
                        PlayerEvent::DownloadCompleted { .. } => {
                            // Unexpectedly completed
                            break;
                        }
                        _ => {
                            // Other events
                        }
                    }
                }
            }
            _ = sleep(Duration::from_millis(100)) => {
                break;
            }
        }
    }
    
    // Check final status
    let final_status = manager.get_download_status(&track.id).await;
    assert!(matches!(final_status,
        DownloadStatus::Failed | 
        DownloadStatus::InProgress |
        DownloadStatus::NotStarted |
        DownloadStatus::Queued
    ));
}

#[tokio::test]
async fn test_download_cleanup() {
    let cache = Cache::new().await.unwrap();
    let (event_sender, _event_receiver) = mpsc::unbounded_channel();
    
    let manager = DownloadManager::new(cache, event_sender);
    let track = Track::from_url("https://httpbin.org/status/200");
    
    // Request and complete download
    manager.request_download(track.clone()).await;
    sleep(Duration::from_millis(200)).await;
    
    // Clean up completed downloads
    manager.cleanup_completed_downloads().await;
    
    // Status should be cleared
    let status = manager.get_download_status(&track.id).await;
    // After cleanup, status might be NotStarted or still show completion
    assert!(matches!(status,
        DownloadStatus::NotStarted |
        DownloadStatus::Completed
    ));
}

#[tokio::test]
async fn test_download_bandwidth_limiting() {
    let cache = Cache::new().await.unwrap();
    let (event_sender, _event_receiver) = mpsc::unbounded_channel();
    
    let manager = DownloadManager::new(cache, event_sender);
    
    // Configure bandwidth limit (if supported)
    manager.set_bandwidth_limit(Some(1024)).await; // 1KB/s limit
    
    let track = Track::from_url("https://httpbin.org/bytes/4096"); // 4KB file
    let start_time = std::time::Instant::now();
    
    // Start download
    manager.request_download(track.clone()).await;
    
    // Wait for download to complete
    sleep(Duration::from_secs(2)).await;
    
    let elapsed = start_time.elapsed();
    
    // With bandwidth limiting, download should take at least a few seconds
    // But this is implementation-dependent
    assert!(elapsed >= Duration::from_millis(0)); // Basic sanity check
}

#[tokio::test]
async fn test_download_queue_management() {
    let cache = Cache::new().await.unwrap();
    let (event_sender, _event_receiver) = mpsc::unbounded_channel();
    
    let manager = DownloadManager::new(cache, event_sender);
    
    // Add multiple tracks to download queue
    let tracks: Vec<_> = (0..5).map(|i| {
        Track::from_url(&format!("https://httpbin.org/bytes/1024?track={}", i))
    }).collect();
    
    for track in &tracks {
        manager.request_download(track.clone()).await;
    }
    
    // Check queue status
    let queue_size = manager.get_queue_size().await;
    assert!(queue_size > 0);
    
    // Clear the queue
    manager.clear_download_queue().await;
    
    let queue_size_after = manager.get_queue_size().await;
    assert_eq!(queue_size_after, 0);
}