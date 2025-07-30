use tempfile::NamedTempFile;
use tokio::fs;

use dab_rs::cache::{Cache, SearchQuery, Id3Metadata};

#[tokio::test]
async fn test_cache_metadata_extraction() {
    // Create a temporary MP3 file with some ID3 metadata
    let mut temp_file = NamedTempFile::new().unwrap();
    let temp_path = temp_file.path();
    
    // Create a basic MP3 file with ID3 tags
    // This is a minimal MP3 file with ID3v2 tags
    let id3_data = vec![
        // ID3v2 header
        0x49, 0x44, 0x33, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3C,
        // Title frame
        0x54, 0x49, 0x54, 0x32, 0x00, 0x00, 0x00, 0x0B, 0x00, 0x00,
        0x54, 0x65, 0x73, 0x74, 0x20, 0x53, 0x6F, 0x6E, 0x67,
        // Artist frame
        0x54, 0x50, 0x45, 0x31, 0x00, 0x00, 0x00, 0x0F, 0x00, 0x00,
        0x54, 0x65, 0x73, 0x74, 0x20, 0x41, 0x72, 0x74, 0x69, 0x73, 0x74,
        // Album frame
        0x54, 0x41, 0x4C, 0x42, 0x00, 0x00, 0x00, 0x0F, 0x00, 0x00,
        0x54, 0x65, 0x73, 0x74, 0x20, 0x41, 0x6C, 0x62, 0x75, 0x6D, 0x00,
        // Simple MP3 frame (just a minimal frame)
        0xFF, 0xFB, 0x90, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    
    fs::write(temp_path, id3_data).await.unwrap();
    
    // Create cache and store the file
    let mut cache = Cache::new().await.unwrap();
    let track_id = "test_track_1";
    let cached_path = cache.store_track(track_id, temp_path).await.unwrap();
    
    // Verify the file was cached
    assert!(cached_path.exists());
    
    // Check if track exists in cache
    assert!(cache.has_track(track_id).await.unwrap());
    
    // Get the metadata
    let metadata = cache.get_track_metadata(track_id).await.unwrap();
    assert!(metadata.is_some());
    
    let metadata = metadata.unwrap();
    assert_eq!(metadata.title, Some("Test Song".to_string()));
    assert_eq!(metadata.artist, Some("Test Artist".to_string()));
    assert_eq!(metadata.album, Some("Test Album".to_string()));
    
    // Test search functionality
    let search_results = cache.search_cached_tracks(&SearchQuery::Title("test".to_string())).await.unwrap();
    assert_eq!(search_results.len(), 1);
    assert_eq!(search_results[0].track_id, track_id);
    
    let artist_results = cache.search_cached_tracks(&SearchQuery::Artist("artist".to_string())).await.unwrap();
    assert_eq!(artist_results.len(), 1);
    
    let album_results = cache.search_cached_tracks(&SearchQuery::Album("album".to_string())).await.unwrap();
    assert_eq!(album_results.len(), 1);
    
    // Test album functionality
    let album_tracks = cache.get_album_tracks("Test Album").await.unwrap();
    assert_eq!(album_tracks.len(), 1);
    
    let albums = cache.get_cached_albums().await.unwrap();
    assert_eq!(albums.len(), 1);
    assert_eq!(albums[0].name, "Test Album");
    
    // Clean up
    cache.remove_track(track_id).await.unwrap();
    assert!(!cache.has_track(track_id).await.unwrap());
}

#[tokio::test]
async fn test_cache_unique_id_generation() {
    let mut cache = Cache::new().await.unwrap();
    
    // Create two temporary files with same content but different metadata
    let temp_file1 = NamedTempFile::new().unwrap();
    let temp_path1 = temp_file1.path();
    
    let temp_file2 = NamedTempFile::new().unwrap();
    let temp_path2 = temp_file2.path();
    
    // Write some dummy content
    fs::write(temp_path1, b"test content 1").await.unwrap();
    fs::write(temp_path2, b"test content 2").await.unwrap();
    
    // Store both files
    let track_id1 = "test_track_1";
    let track_id2 = "test_track_2";
    
    cache.store_track(track_id1, temp_path1).await.unwrap();
    cache.store_track(track_id2, temp_path2).await.unwrap();
    
    // Get unique IDs
    let entry1 = cache.metadata.entries.get(track_id1).unwrap();
    let entry2 = cache.metadata.entries.get(track_id2).unwrap();
    
    // Unique IDs should be different
    assert_ne!(entry1.unique_id, entry2.unique_id);
    
    // Test lookup by unique ID
    let found_track_id = cache.get_track_by_unique_id(&entry1.unique_id).await.unwrap();
    assert_eq!(found_track_id, Some(track_id1.to_string()));
    
    assert!(cache.has_track_by_unique_id(&entry1.unique_id).await.unwrap());
    assert!(!cache.has_track_by_unique_id("nonexistent_id").await.unwrap());
    
    // Clean up
    cache.remove_track(track_id1).await.unwrap();
    cache.remove_track(track_id2).await.unwrap();
}

#[tokio::test]
async fn test_cache_metadata_update() {
    let mut cache = Cache::new().await.unwrap();
    
    // Create a temporary file
    let temp_file = NamedTempFile::new().unwrap();
    let temp_path = temp_file.path();
    fs::write(temp_path, b"test content").await.unwrap();
    
    // Store the file
    let track_id = "test_track_update";
    cache.store_track(track_id, temp_path).await.unwrap();
    
    // Get original metadata
    let original_metadata = cache.get_track_metadata(track_id).await.unwrap();
    
    // Update metadata
    let new_metadata = Id3Metadata {
        title: Some("Updated Title".to_string()),
        artist: Some("Updated Artist".to_string()),
        album: Some("Updated Album".to_string()),
        year: Some(2024),
        genre: Some("Rock".to_string()),
        duration_ms: Some(180000),
        track_number: Some(1),
        total_tracks: Some(10),
        album_artist: Some("Updated Album Artist".to_string()),
        composer: None,
        comment: None,
        cover_art_hash: None,
    };
    
    cache.update_track_metadata(track_id, new_metadata.clone()).await.unwrap();
    
    // Verify the update
    let updated_metadata = cache.get_track_metadata(track_id).await.unwrap();
    assert_eq!(updated_metadata, Some(new_metadata));
    
    // Clean up
    cache.remove_track(track_id).await.unwrap();
}