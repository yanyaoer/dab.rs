use dab::{Cache, CacheStatus, PlayerEvent, Queue, Track};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]  
    async fn test_cache_basic_functionality() {
        let mut cache = Cache::new().await.unwrap();
        
        // Initially not cached
        let status = cache.get_cache_status("test_track").await;
        assert_eq!(status, CacheStatus::NotCached);
        
        // Test basic operations work
        let has_track = cache.has_track("test_track").await.unwrap();
        assert!(!has_track);
        
        // Test progress methods work without error
        let progress = cache.get_download_progress("test_track").await;
        // Progress might be Some(0.0) for tracks that don't exist yet, so just check it doesn't panic
        let _ = progress;
        
        // Test stream ready check (it might be true or false, just ensure it doesn't panic)
        let _is_ready = cache.is_stream_ready("test_track", 5).await;
        
        println!("Cache basic functionality test passed");
    }

    #[tokio::test]
    async fn test_player_events_flow() {
        // Test that we can create the new events
        let track = Track::from_url("http://example.com/test.mp3");
        
        let events = vec![
            PlayerEvent::DownloadStarted { track_id: track.id.clone() },
            PlayerEvent::DownloadProgress { track_id: track.id.clone(), progress: 0.5 },
            PlayerEvent::StreamReady { track_id: track.id.clone() },
            PlayerEvent::BufferingStart { track_id: track.id.clone() },
            PlayerEvent::BufferingEnd { track_id: track.id.clone() },
            PlayerEvent::DownloadCompleted { track_id: track.id.clone() },
        ];
        
        // All events should be creatable
        assert_eq!(events.len(), 6);
        
        // Events should be cloneable
        let _cloned_events = events.clone();
    }

    #[tokio::test]
    async fn test_queue_peek_functionality() {
        let queue = Queue::new();
        
        // Add some tracks
        let track1 = Track::from_url("http://example.com/track1.mp3");
        let track2 = Track::from_url("http://example.com/track2.mp3");
        let track3 = Track::from_url("http://example.com/track3.mp3");
        
        queue.add_track(track1.clone()).await;
        queue.add_track(track2.clone()).await;
        queue.add_track(track3.clone()).await;
        
        // Wait for tracks to be added
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        // Start by getting the first track to set current index
        let _first = queue.next_track().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        
        // Now peek at next tracks should work
        let next_tracks = queue.peek_next_n(2).await.unwrap();
        println!("Next tracks count: {}", next_tracks.len());
        
        // Should have 2 remaining tracks (track2 and track3)
        assert_eq!(next_tracks.len(), 2);
        if next_tracks.len() >= 2 {
            assert_eq!(next_tracks[0].id, track2.id);
            assert_eq!(next_tracks[1].id, track3.id);
        }
    }

    #[tokio::test]
    async fn test_track_creation_and_properties() {
        // Test local track
        let local_track = Track::from_url("/path/to/local/file.mp3");
        assert!(local_track.is_local());
        assert!(!local_track.requires_stream_url());
        
        // Test URL track  
        let url_track = Track::from_url("http://example.com/remote.mp3");
        assert!(!url_track.is_local());
        assert!(!url_track.requires_stream_url()); // No track_id set
        
        // Test streaming integration
        assert_ne!(local_track.id, url_track.id);
        assert_eq!(local_track.title, "file");
        assert_eq!(url_track.title, "remote");
    }
}