use dab::player::{Queue, QueueCommand, RepeatMode, Track};
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn test_queue_basic_operations() {
    let queue = Queue::new();

    // Test empty queue
    assert_eq!(queue.len().await, 0);
    assert!(queue.get_current_track().await.is_none());

    // Add a track
    let track1 = Track::from_url("https://example.com/track1.mp3");
    queue.add_track(track1.clone()).await;

    // Wait for async command to process
    sleep(Duration::from_millis(50)).await;

    assert_eq!(queue.len().await, 1);

    // Add another track
    let track2 = Track::from_url("https://example.com/track2.mp3");
    queue.add_track(track2.clone()).await;

    // Wait for async command to process
    sleep(Duration::from_millis(50)).await;

    assert_eq!(queue.len().await, 2);

    // Test get queue
    let tracks = queue.get_queue().await;
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].id, track1.id);
    assert_eq!(tracks[1].id, track2.id);
}

#[tokio::test]
async fn test_queue_navigation() {
    let queue = Queue::new();

    // Add test tracks
    let track1 = Track::from_url("https://example.com/track1.mp3");
    let track2 = Track::from_url("https://example.com/track2.mp3");
    let track3 = Track::from_url("https://example.com/track3.mp3");

    queue.add_track(track1.clone()).await;
    queue.add_track(track2.clone()).await;
    queue.add_track(track3.clone()).await;

    // Wait for async commands to process
    sleep(Duration::from_millis(50)).await;

    // Navigate through queue
    let current = queue.next_track().await;
    sleep(Duration::from_millis(10)).await;
    assert!(current.is_some());
    assert_eq!(current.unwrap().id, track1.id);

    let current = queue.next_track().await;
    sleep(Duration::from_millis(10)).await;
    assert!(current.is_some());
    assert_eq!(current.unwrap().id, track2.id);

    let current = queue.next_track().await;
    sleep(Duration::from_millis(10)).await;
    assert!(current.is_some());
    assert_eq!(current.unwrap().id, track3.id);

    // At end of queue without repeat
    let current = queue.next_track().await;
    sleep(Duration::from_millis(10)).await;
    assert!(current.is_none());

    // Check current index after reaching end
    let current_idx = queue.get_current_index().await;
    println!("Current index after end: {:?}", current_idx);

    // Go back - from None/end position to the last track in the queue
    let current = queue.previous_track().await;
    sleep(Duration::from_millis(10)).await;
    println!(
        "After previous_track: {:?}",
        current.as_ref().map(|t| &t.id)
    );
    let current_idx = queue.get_current_index().await;
    println!("Current index after previous: {:?}", current_idx);

    assert!(current.is_some());
    // When at the end (None), previous should go to the last track (track3)
    assert_eq!(current.unwrap().id, track3.id);
}

#[tokio::test]
async fn test_queue_add_next() {
    let queue = Queue::new();

    let track1 = Track::from_url("https://example.com/track1.mp3");
    let track2 = Track::from_url("https://example.com/track2.mp3");
    let track3 = Track::from_url("https://example.com/track3.mp3");

    queue.add_track(track1.clone()).await;
    queue.add_track(track2.clone()).await;

    // Wait for async commands to process
    sleep(Duration::from_millis(50)).await;

    // Set current to first track
    queue.next_track().await;
    sleep(Duration::from_millis(10)).await;

    // Add track3 next
    queue.add_track_next(track3.clone()).await;
    sleep(Duration::from_millis(50)).await;

    let tracks = queue.get_queue().await;
    assert_eq!(tracks.len(), 3);
    // track3 should be inserted after track1 (current position)
    assert_eq!(tracks[1].id, track3.id);
}

#[tokio::test]
async fn test_queue_commands() {
    let queue = Queue::new();

    let track1 = Track::from_url("https://example.com/track1.mp3");
    let track2 = Track::from_url("https://example.com/track2.mp3");

    // Test add command
    let _ = queue
        .send_command(QueueCommand::AddTrack(track1.clone()))
        .await;
    let _ = queue
        .send_command(QueueCommand::AddTrack(track2.clone()))
        .await;

    // Give the command some time to process
    sleep(Duration::from_millis(50)).await;

    assert_eq!(queue.len().await, 2);

    // Test clear command
    let _ = queue.send_command(QueueCommand::Clear).await;
    sleep(Duration::from_millis(50)).await;

    assert_eq!(queue.len().await, 0);
}

#[tokio::test]
async fn test_queue_repeat_mode() {
    let queue = Queue::new();

    let track1 = Track::from_url("https://example.com/track1.mp3");
    let track2 = Track::from_url("https://example.com/track2.mp3");

    queue.add_track(track1.clone()).await;
    queue.add_track(track2.clone()).await;

    // Wait for async commands to process
    sleep(Duration::from_millis(50)).await;

    // Set repeat mode to All
    let _ = queue
        .send_command(QueueCommand::SetRepeat(RepeatMode::All))
        .await;
    sleep(Duration::from_millis(50)).await;

    // Navigate to end and then next should wrap around
    queue.next_track().await; // track1
    sleep(Duration::from_millis(10)).await;
    queue.next_track().await; // track2
    sleep(Duration::from_millis(10)).await;

    let current = queue.next_track().await; // should wrap to track1
    sleep(Duration::from_millis(10)).await;
    assert!(current.is_some());
    assert_eq!(current.unwrap().id, track1.id);
}

#[tokio::test]
async fn test_track_from_url() {
    let url = "https://example.com/music/artist_name-track_title.mp3";
    let track = Track::from_url(url);

    assert_eq!(track.local_path, None); // HTTP URLs are not treated as local paths
    assert_eq!(track.title, "artist_name-track_title");
    assert_eq!(track.artist, "Unknown Artist");
    assert_eq!(track.album, "Unknown Album");
    assert!(!track.id.is_empty());
}
