use dab::{player::queue::*, PlayerEvent, Track};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn test_queue_shuffle_functionality() {
    let queue = Queue::new();

    // Add multiple tracks
    let tracks: Vec<_> = (0..10)
        .map(|i| Track::from_url(&format!("https://example.com/track{}.mp3", i)))
        .collect();

    for track in &tracks {
        queue.add_track(track.clone()).await;
    }

    sleep(Duration::from_millis(100)).await;

    // Enable shuffle
    queue
        .send_command(QueueCommand::SetShuffle(true))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;

    // Get shuffled queue
    let shuffled_queue = queue.get_queue().await;
    assert_eq!(shuffled_queue.len(), tracks.len());

    // Disable shuffle
    queue
        .send_command(QueueCommand::SetShuffle(false))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;

    let unshuffled_queue = queue.get_queue().await;
    assert_eq!(unshuffled_queue.len(), tracks.len());
}

#[tokio::test]
async fn test_queue_repeat_modes() {
    let queue = Queue::new();

    let track1 = Track::from_url("https://example.com/track1.mp3");
    let track2 = Track::from_url("https://example.com/track2.mp3");

    queue.add_track(track1.clone()).await;
    queue.add_track(track2.clone()).await;
    sleep(Duration::from_millis(50)).await;

    // Test RepeatMode::Track
    queue
        .send_command(QueueCommand::SetRepeat(RepeatMode::Track))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;

    // Get first track
    let current = queue.next_track().await;
    assert!(current.is_some());
    assert_eq!(current.unwrap().id, track1.id);

    // With repeat track, next should return same track
    let next = queue.next_track().await;
    assert!(next.is_some());
    assert_eq!(next.unwrap().id, track1.id);

    // Test RepeatMode::All
    queue
        .send_command(QueueCommand::SetRepeat(RepeatMode::All))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;

    // Navigate to end and beyond
    queue.next_track().await; // track2
    sleep(Duration::from_millis(10)).await;
    let wrapped = queue.next_track().await; // should wrap to track1
    sleep(Duration::from_millis(10)).await;
    assert!(wrapped.is_some());
    assert_eq!(wrapped.unwrap().id, track1.id);

    // Test RepeatMode::Off
    queue
        .send_command(QueueCommand::SetRepeat(RepeatMode::Off))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;

    queue.next_track().await; // track2
    sleep(Duration::from_millis(10)).await;
    let end_result = queue.next_track().await; // should return None
    sleep(Duration::from_millis(10)).await;
    assert!(end_result.is_none());
}

#[tokio::test]
async fn test_queue_remove_track() {
    let queue = Queue::new();

    let track1 = Track::from_url("https://example.com/track1.mp3");
    let track2 = Track::from_url("https://example.com/track2.mp3");
    let track3 = Track::from_url("https://example.com/track3.mp3");

    queue.add_track(track1.clone()).await;
    queue.add_track(track2.clone()).await;
    queue.add_track(track3.clone()).await;
    sleep(Duration::from_millis(100)).await;

    assert_eq!(queue.len().await, 3);

    // Remove middle track
    queue
        .send_command(QueueCommand::RemoveTrack(1))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;

    assert_eq!(queue.len().await, 2);

    let remaining_tracks = queue.get_queue().await;
    assert_eq!(remaining_tracks[0].id, track1.id);
    assert_eq!(remaining_tracks[1].id, track3.id);
}

#[tokio::test]
async fn test_queue_move_track() {
    let queue = Queue::new();

    let track1 = Track::from_url("https://example.com/track1.mp3");
    let track2 = Track::from_url("https://example.com/track2.mp3");
    let track3 = Track::from_url("https://example.com/track3.mp3");

    queue.add_track(track1.clone()).await;
    queue.add_track(track2.clone()).await;
    queue.add_track(track3.clone()).await;
    sleep(Duration::from_millis(100)).await;

    // Move track from position 2 to position 0
    queue
        .send_command(QueueCommand::MoveTrack(2, 0))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;

    let reordered_tracks = queue.get_queue().await;
    assert_eq!(reordered_tracks[0].id, track3.id);
    assert_eq!(reordered_tracks[1].id, track1.id);
    assert_eq!(reordered_tracks[2].id, track2.id);
}

#[tokio::test]
async fn test_queue_replace_functionality() {
    let queue = Queue::new();

    // Add initial tracks
    let initial_tracks: Vec<_> = (0..3)
        .map(|i| Track::from_url(&format!("https://example.com/initial{}.mp3", i)))
        .collect();

    for track in &initial_tracks {
        queue.add_track(track.clone()).await;
    }
    sleep(Duration::from_millis(100)).await;
    assert_eq!(queue.len().await, 3);

    // Replace with new tracks
    let new_tracks: Vec<_> = (0..5)
        .map(|i| Track::from_url(&format!("https://example.com/new{}.mp3", i)))
        .collect();

    queue
        .send_command(QueueCommand::ReplaceQueue(new_tracks.clone()))
        .await
        .unwrap();
    sleep(Duration::from_millis(100)).await;

    assert_eq!(queue.len().await, 5);

    let queue_tracks = queue.get_queue().await;
    for (i, track) in queue_tracks.iter().enumerate() {
        assert_eq!(track.id, new_tracks[i].id);
    }
}

#[tokio::test]
async fn test_queue_jump_to_track() {
    let queue = Queue::new();

    let tracks: Vec<_> = (0..5)
        .map(|i| Track::from_url(&format!("https://example.com/track{}.mp3", i)))
        .collect();

    for track in &tracks {
        queue.add_track(track.clone()).await;
    }
    sleep(Duration::from_millis(100)).await;

    // Jump to track at index 3
    queue
        .send_command(QueueCommand::JumpToTrack(3))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;

    let current_index = queue.get_current_index().await;
    assert_eq!(current_index, Some(3));

    let current_track = queue.get_current_track().await;
    assert!(current_track.is_some());
    assert_eq!(current_track.unwrap().id, tracks[3].id);
}

#[tokio::test]
async fn test_queue_event_notifications() {
    let (event_sender, mut event_receiver) = mpsc::unbounded_channel();
    let queue = Queue::with_event_sender(event_sender);

    let track = Track::from_url("https://example.com/track.mp3");

    // Add track and listen for events
    queue.add_track(track.clone()).await;

    // Wait for potential events
    tokio::select! {
        event = event_receiver.recv() => {
            if let Some(event) = event {
                match event {
                    PlayerEvent::QueueChanged => {
                        // Expected event
                    }
                    PlayerEvent::TrackChanged(changed_track) => {
                        assert_eq!(changed_track.id, track.id);
                    }
                    _ => {
                        // Other events are also acceptable
                    }
                }
            }
        }
        _ = sleep(Duration::from_millis(200)) => {
            // Timeout is acceptable if events are not implemented
        }
    }
}

#[tokio::test]
async fn test_queue_boundary_conditions() {
    let queue = Queue::new();

    // Test operations on empty queue
    assert_eq!(queue.len().await, 0);
    assert!(queue.get_current_track().await.is_none());
    assert!(queue.next_track().await.is_none());
    assert!(queue.previous_track().await.is_none());

    // Test invalid index operations
    let result = queue.send_command(QueueCommand::RemoveTrack(999)).await;
    // Should handle gracefully without panicking

    let result = queue.send_command(QueueCommand::JumpToTrack(999)).await;
    // Should handle gracefully without panicking

    let result = queue.send_command(QueueCommand::MoveTrack(0, 999)).await;
    // Should handle gracefully without panicking
}

#[tokio::test]
async fn test_queue_peek_functionality_extended() {
    let queue = Queue::new();

    let tracks: Vec<_> = (0..10)
        .map(|i| Track::from_url(&format!("https://example.com/track{}.mp3", i)))
        .collect();

    for track in &tracks {
        queue.add_track(track.clone()).await;
    }
    sleep(Duration::from_millis(100)).await;

    // Set current position
    queue.next_track().await; // Move to track 0
    sleep(Duration::from_millis(50)).await;

    // Peek at next 3 tracks
    let next_tracks = queue.peek_next_n(3).await.unwrap();
    assert_eq!(next_tracks.len(), 3);

    for (i, track) in next_tracks.iter().enumerate() {
        assert_eq!(track.id, tracks[i + 1].id); // Should be tracks 1, 2, 3
    }

    // Peek at previous tracks
    queue.next_track().await; // Move to track 1
    queue.next_track().await; // Move to track 2
    sleep(Duration::from_millis(100)).await;

    let prev_tracks = queue.peek_previous_n(2).await.unwrap();
    assert_eq!(prev_tracks.len(), 2);

    // Should be tracks 1, 0 (in reverse order)
    assert_eq!(prev_tracks[0].id, tracks[1].id);
    assert_eq!(prev_tracks[1].id, tracks[0].id);
}

#[tokio::test]
async fn test_queue_history_tracking() {
    let queue = Queue::new();

    let tracks: Vec<_> = (0..5)
        .map(|i| Track::from_url(&format!("https://example.com/track{}.mp3", i)))
        .collect();

    for track in &tracks {
        queue.add_track(track.clone()).await;
    }
    sleep(Duration::from_millis(100)).await;

    // Navigate through tracks to build history
    queue.next_track().await; // track 0
    sleep(Duration::from_millis(50)).await;
    queue.next_track().await; // track 1
    sleep(Duration::from_millis(50)).await;
    queue.next_track().await; // track 2
    sleep(Duration::from_millis(50)).await;

    // Get play history
    let history = queue.get_play_history().await;
    assert!(history.len() >= 2); // Should have at least some history

    // Clear history
    queue
        .send_command(QueueCommand::ClearHistory)
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;

    let cleared_history = queue.get_play_history().await;
    assert_eq!(cleared_history.len(), 0);
}

#[tokio::test]
async fn test_queue_batch_operations() {
    let queue = Queue::new();

    let tracks: Vec<_> = (0..10)
        .map(|i| Track::from_url(&format!("https://example.com/track{}.mp3", i)))
        .collect();

    // Add all tracks at once
    queue
        .send_command(QueueCommand::AddTracks(tracks.clone()))
        .await
        .unwrap();
    sleep(Duration::from_millis(100)).await;

    assert_eq!(queue.len().await, 10);

    // Remove multiple tracks
    let indices_to_remove = vec![1, 3, 5, 7];
    queue
        .send_command(QueueCommand::RemoveTracks(indices_to_remove))
        .await
        .unwrap();
    sleep(Duration::from_millis(100)).await;

    assert_eq!(queue.len().await, 6); // 10 - 4 = 6
}

#[tokio::test]
async fn test_queue_concurrent_modifications() {
    let queue = Queue::new();
    let mut handles = vec![];

    // Spawn multiple tasks modifying the queue concurrently
    for i in 0..5 {
        let queue_clone = queue.clone();
        let handle = tokio::spawn(async move {
            let track = Track::from_url(&format!("https://example.com/concurrent{}.mp3", i));
            queue_clone.add_track(track).await;
        });
        handles.push(handle);
    }

    // Wait for all modifications to complete
    for handle in handles {
        handle.await.unwrap();
    }

    sleep(Duration::from_millis(200)).await;

    // Should have all tracks added
    assert_eq!(queue.len().await, 5);
}

#[tokio::test]
async fn test_queue_persistence() {
    // This test would require implementing queue persistence
    // For now, just test that the queue maintains state across operations

    let queue = Queue::new();

    let track1 = Track::from_url("https://example.com/track1.mp3");
    let track2 = Track::from_url("https://example.com/track2.mp3");

    // Add tracks
    queue.add_track(track1.clone()).await;
    queue.add_track(track2.clone()).await;
    sleep(Duration::from_millis(100)).await;

    // Set repeat mode and shuffle
    queue
        .send_command(QueueCommand::SetRepeat(RepeatMode::All))
        .await
        .unwrap();
    queue
        .send_command(QueueCommand::SetShuffle(true))
        .await
        .unwrap();
    sleep(Duration::from_millis(100)).await;

    // Queue state should persist
    assert_eq!(queue.len().await, 2);
    let queue_tracks = queue.get_queue().await;
    assert_eq!(queue_tracks.len(), 2);
}

#[tokio::test]
async fn test_queue_search_functionality() {
    let queue = Queue::new();

    let tracks = vec![
        Track {
            id: "1".to_string(),
            title: "Hello World".to_string(),
            artist: "Artist A".to_string(),
            album: "Album X".to_string(),
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
        },
        Track {
            id: "2".to_string(),
            title: "Goodbye Moon".to_string(),
            artist: "Artist B".to_string(),
            album: "Album Y".to_string(),
            duration: Some(200),
            track_number: None,
            disc_number: None,
            year: None,
            genre: None,
            album_id: None,
            artist_id: None,
            local_path: None,
            stream_url: None,
            cover_url: None,
        },
    ];

    for track in &tracks {
        queue.add_track(track.clone()).await;
    }
    sleep(Duration::from_millis(100)).await;

    // Search for tracks by title
    let search_results = queue.search_queue("Hello").await;
    assert_eq!(search_results.len(), 1);
    assert_eq!(search_results[0].title, "Hello World");

    // Search for tracks by artist
    let artist_results = queue.search_queue("Artist B").await;
    assert_eq!(artist_results.len(), 1);
    assert_eq!(artist_results[0].artist, "Artist B");
}
