use dab::cache::Cache;
use dab::library::Library;
use dab::search::DabAlbum;
use std::env;

#[tokio::test]
async fn test_favorite_album_persistence() {
    // Use unique cache directory for this test
    env::set_var("HOME", "/tmp/dab_test_cache_persistence");

    // Clean up any existing cache from previous runs
    if let Ok(cache_dir) = std::fs::read_dir("/tmp/dab_test_cache_persistence") {
        for entry in cache_dir.flatten() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }

    // Create a mock album
    let test_album = DabAlbum {
        id: "test-album-123".to_string(),
        title: "Test Album".to_string(),
        artist: "Test Artist".to_string(),
        artist_id: Some("test-artist-456".to_string()),
        release_date: Some("2023-01-01".to_string()),
        genre: Some("Rock".to_string()),
        cover: Some("https://example.com/cover.jpg".to_string()),
        tracks: None,
        track_count: Some(10),
        duration: Some(3600),
        label: None,
        upc: None,
        url: None,
        streamable: None,
        downloadable: None,
        media_count: None,
        maximum_channel_count: None,
        parental_warning: None,
        popularity: None,
        audio_quality: None,
    };

    // Test adding favorite album
    {
        let cache = Cache::new().await.unwrap();
        let mut library = Library::new(cache).await.unwrap();

        // Add the album to favorites
        let is_new = library.add_favorite_album(&test_album).unwrap();
        assert!(is_new);

        // Try adding the same album again
        let is_duplicate = library.add_favorite_album(&test_album).unwrap();
        assert!(!is_duplicate);

        // Check if album is in favorites
        assert!(library.is_favorite_album(&test_album.artist, &test_album.title));

        // Get favorites and verify
        let favorites = library.get_favorite_albums();
        assert_eq!(favorites.len(), 1);
        assert_eq!(favorites[0].id, test_album.id);
        assert_eq!(favorites[0].title, test_album.title);
        assert_eq!(favorites[0].artist, test_album.artist);
    }

    // Test persistence by creating a new library instance
    {
        let cache = Cache::new().await.unwrap();
        let library = Library::new(cache).await.unwrap();

        // Check if album persisted across restarts
        let favorites = library.get_favorite_albums();
        assert_eq!(favorites.len(), 1);
        assert_eq!(favorites[0].id, test_album.id);
        assert_eq!(favorites[0].title, test_album.title);
        assert_eq!(favorites[0].artist, test_album.artist);
    }
}

#[tokio::test]
async fn test_remove_favorite_album() {
    // Use unique cache directory for this test
    env::set_var("HOME", "/tmp/dab_test_cache_remove_test");

    // Clean up any existing cache from previous runs
    if let Ok(cache_dir) = std::fs::read_dir("/tmp/dab_test_cache_remove_test") {
        for entry in cache_dir.flatten() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }

    let cache = Cache::new().await.unwrap();
    let mut library = Library::new(cache).await.unwrap();

    let test_album = DabAlbum {
        id: "test-album-789".to_string(),
        title: "Another Test Album".to_string(),
        artist: "Another Test Artist".to_string(),
        artist_id: Some("test-artist-890".to_string()),
        release_date: Some("2023-02-01".to_string()),
        genre: Some("Pop".to_string()),
        cover: None,
        tracks: None,
        track_count: Some(8),
        duration: Some(2400),
        label: None,
        upc: None,
        url: None,
        streamable: None,
        downloadable: None,
        media_count: None,
        maximum_channel_count: None,
        parental_warning: None,
        popularity: None,
        audio_quality: None,
    };

    // Add album
    library.add_favorite_album(&test_album).unwrap();
    assert_eq!(library.get_favorite_albums().len(), 1);

    // Remove album
    let was_removed = library
        .remove_favorite_album(&test_album.artist, &test_album.title)
        .unwrap();
    assert!(was_removed);
    assert_eq!(library.get_favorite_albums().len(), 0);

    // Try removing again
    let was_removed_again = library
        .remove_favorite_album(&test_album.artist, &test_album.title)
        .unwrap();
    assert!(!was_removed_again);
}
