use dab::{player::*, DabError, Track};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn test_player_engine_initialization() {
    let engine = PlayerEngine::new().unwrap();

    // Test initial state
    let state = engine.get_state().await;
    assert_eq!(state, PlayerState::Stopped);

    let current_track = engine.get_current_track().await;
    assert!(current_track.is_none());

    let position = engine.get_position().await;
    assert_eq!(position, Duration::from_secs(0));
}

#[tokio::test]
async fn test_player_commands() {
    let engine = PlayerEngine::new().unwrap();

    // Test that commands can be sent without panicking
    let track = Track::from_url("https://example.com/test.mp3");

    // Test load command
    engine
        .send_command(PlayerCommand::LoadAndPlayTrack(track.clone()))
        .await
        .unwrap();
    sleep(Duration::from_millis(100)).await;

    // Test play/pause commands
    engine.send_command(PlayerCommand::Play).await.unwrap();
    sleep(Duration::from_millis(50)).await;

    engine.send_command(PlayerCommand::Pause).await.unwrap();
    sleep(Duration::from_millis(50)).await;

    // Test stop command
    engine.send_command(PlayerCommand::Stop).await.unwrap();
    sleep(Duration::from_millis(50)).await;

    let state = engine.get_state().await;
    assert_eq!(state, PlayerState::Stopped);
}

#[tokio::test]
async fn test_volume_control() {
    let engine = PlayerEngine::new().unwrap();

    // Test initial volume
    let initial_volume = engine.get_volume().await;
    assert!((0.0..=1.0).contains(&initial_volume));

    // Test volume change
    let test_volume = 0.7;
    engine
        .send_command(PlayerCommand::SetVolume(test_volume))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;

    let current_volume = engine.get_volume().await;
    assert!((test_volume - 0.01..=test_volume + 0.01).contains(&current_volume));
}

#[tokio::test]
async fn test_player_events() {
    let engine = PlayerEngine::new().unwrap();
    let mut event_receiver = engine.subscribe_to_events().await;

    // Create a track and send load command
    let track = Track::from_url("https://example.com/test.mp3");
    engine
        .send_command(PlayerCommand::LoadAndPlayTrack(track.clone()))
        .await
        .unwrap();

    // Wait for potential events
    tokio::select! {
        event = event_receiver.recv() => {
            // If we get an event, it should be valid
            if let Ok(event) = event {
                match event {
                    PlayerEvent::StateChanged(_) |
                    PlayerEvent::TrackChanged(_) |
                    PlayerEvent::VolumeChanged(_) |
                    PlayerEvent::PositionChanged(_) |
                    PlayerEvent::DownloadStarted { .. } |
                    PlayerEvent::DownloadProgress { .. } |
                    PlayerEvent::StreamReady { .. } |
                    PlayerEvent::BufferingStart { .. } |
                    PlayerEvent::BufferingEnd { .. } |
                    PlayerEvent::DownloadCompleted { .. } => {
                        // Event is valid
                    }
                }
            }
        }
        _ = sleep(Duration::from_millis(500)) => {
            // Timeout is fine, events are asynchronous
        }
    }
}

#[tokio::test]
async fn test_seek_functionality() {
    let engine = PlayerEngine::new().unwrap();

    // Test seek command
    let seek_position = Duration::from_secs(30);
    engine
        .send_command(PlayerCommand::Seek(seek_position))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;

    // The actual seek behavior depends on whether a track is loaded
    // but the command should not cause errors
}

#[tokio::test]
async fn test_queue_integration() {
    let engine = PlayerEngine::new().unwrap();

    let track1 = Track::from_url("https://example.com/track1.mp3");
    let track2 = Track::from_url("https://example.com/track2.mp3");

    // Test adding tracks to queue
    engine
        .send_command(PlayerCommand::AddTrackToQueue(track1.clone()))
        .await
        .unwrap();
    engine
        .send_command(PlayerCommand::AddTrackToQueue(track2.clone()))
        .await
        .unwrap();
    sleep(Duration::from_millis(100)).await;

    // Test queue navigation
    engine.send_command(PlayerCommand::NextTrack).await.unwrap();
    sleep(Duration::from_millis(50)).await;

    engine
        .send_command(PlayerCommand::PreviousTrack)
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn test_repeat_mode() {
    let engine = PlayerEngine::new().unwrap();

    // Test setting different repeat modes
    engine
        .send_command(PlayerCommand::SetRepeat(RepeatMode::Off))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;

    engine
        .send_command(PlayerCommand::SetRepeat(RepeatMode::Track))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;

    engine
        .send_command(PlayerCommand::SetRepeat(RepeatMode::All))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn test_shuffle_mode() {
    let engine = PlayerEngine::new().unwrap();

    // Test shuffle toggle
    engine
        .send_command(PlayerCommand::SetShuffle(false))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;

    engine
        .send_command(PlayerCommand::SetShuffle(true))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn test_player_state_transitions() {
    let engine = PlayerEngine::new().unwrap();

    // Initial state should be Stopped
    let state = engine.get_state().await;
    assert_eq!(state, PlayerState::Stopped);

    // Load a track
    let track = Track::from_url("https://example.com/test.mp3");
    engine
        .send_command(PlayerCommand::LoadAndPlayTrack(track))
        .await
        .unwrap();
    sleep(Duration::from_millis(100)).await;

    // State might change to Loading, Playing, or stay as Stopped (depending on implementation)
    let state = engine.get_state().await;
    assert!(matches!(
        state,
        PlayerState::Stopped | PlayerState::Loading | PlayerState::Playing | PlayerState::Buffering
    ));
}

#[tokio::test]
async fn test_concurrent_commands() {
    let engine = PlayerEngine::new().unwrap();
    let engine_arc = Arc::new(Mutex::new(engine));

    let mut handles = vec![];

    // Send multiple commands concurrently
    for i in 0..5 {
        let engine_clone = Arc::clone(&engine_arc);
        let handle = tokio::spawn(async move {
            let engine = engine_clone.lock().await;
            let track = Track::from_url(&format!("https://example.com/track{}.mp3", i));
            engine
                .send_command(PlayerCommand::AddTrackToQueue(track))
                .await
                .unwrap();
        });
        handles.push(handle);
    }

    // Wait for all commands to complete
    for handle in handles {
        handle.await.unwrap();
    }

    sleep(Duration::from_millis(200)).await;
}

#[tokio::test]
async fn test_error_handling() {
    let engine = PlayerEngine::new().unwrap();

    // Test with invalid URLs or commands that might fail
    let invalid_track = Track::from_url("invalid://not-a-real-url");

    // This should not panic the engine
    let result = engine
        .send_command(PlayerCommand::LoadAndPlayTrack(invalid_track))
        .await;

    // The engine should handle the error gracefully
    match result {
        Ok(_) => {
            // Command was accepted, actual error handling happens asynchronously
        }
        Err(e) => {
            // Command was rejected, which is also acceptable
            assert!(matches!(e, DabError::PlayerError(_)));
        }
    }
}
