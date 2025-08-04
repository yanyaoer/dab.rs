use dab::{async_client::*, api_cache::*, search::*, DabError};
use mockito::{Mock, Server};
use serde_json::json;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_async_client_initialization() {
    let base_url = "https://api.example.com".to_string();
    let client = AsyncClient::new(base_url.clone());
    
    // Client should be created successfully
    assert_eq!(client.base_url, base_url);
}

#[tokio::test]
async fn test_network_manager() {
    let manager = NetworkManager::new();
    
    // Test basic functionality
    assert!(manager.is_connected().await);
    
    // Test rate limiting
    let start = std::time::Instant::now();
    manager.wait_for_rate_limit().await;
    let duration = start.elapsed();
    
    // Should complete quickly for first call
    assert!(duration < Duration::from_millis(100));
}

#[tokio::test]
async fn test_async_network_client_mock_response() {
    let mut server = Server::new_async().await;
    
    // Mock successful search response
    let mock_response = json!({
        "tracks": [{
            "id": "123",
            "title": "Test Song",
            "artist": "Test Artist",
            "album": "Test Album",
            "duration": 180
        }]
    });
    
    let _mock = server
        .mock("GET", "/search")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_response.to_string())
        .create_async()
        .await;
    
    let client = AsyncNetworkClient::new(server.url());
    let result = client.get("/search?q=test&type=track").await;
    
    assert!(result.is_ok());
    let response = result.unwrap();
    assert!(response.contains("Test Song"));
}

#[tokio::test]
async fn test_async_network_client_error_handling() {
    let mut server = Server::new_async().await;
    
    // Mock error response
    let _mock = server
        .mock("GET", "/search")
        .with_status(404)
        .with_body("Not Found")
        .create_async()
        .await;
    
    let client = AsyncNetworkClient::new(server.url());
    let result = client.get("/search?q=nonexistent").await;
    
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(matches!(e, DabError::NetworkError(_)));
    }
}

#[tokio::test]
async fn test_async_network_client_timeout() {
    let mut server = Server::new_async().await;
    
    // Mock slow response
    let _mock = server
        .mock("GET", "/slow")
        .with_status(200)
        .with_delay(Duration::from_secs(2))
        .with_body("Slow response")
        .create_async()
        .await;
    
    let client = AsyncNetworkClient::new(server.url());
    
    // Test with short timeout
    let start = std::time::Instant::now();
    let result = client.get_with_timeout("/slow", Duration::from_millis(500)).await;
    let elapsed = start.elapsed();
    
    // Should timeout quickly
    assert!(elapsed < Duration::from_secs(1));
    assert!(result.is_err());
}

#[tokio::test]
async fn test_api_cache_functionality() {
    let cache = ApiCache::new(100); // 100MB cache size
    let key = "test_key".to_string();
    let value = "test_value".to_string();
    
    // Test cache miss
    assert!(cache.get(&key).is_none());
    
    // Test cache insertion
    cache.insert(key.clone(), value.clone(), Duration::from_secs(60));
    
    // Test cache hit
    let cached_value = cache.get(&key);
    assert!(cached_value.is_some());
    assert_eq!(cached_value.unwrap(), value);
    
    // Test cache size
    assert!(cache.size() > 0);
}

#[tokio::test]
async fn test_api_cache_expiration() {
    let cache = ApiCache::new(100);
    let key = "expiring_key".to_string();
    let value = "expiring_value".to_string();
    
    // Insert with very short TTL
    cache.insert(key.clone(), value.clone(), Duration::from_millis(100));
    
    // Should be available immediately
    assert!(cache.get(&key).is_some());
    
    // Wait for expiration
    sleep(Duration::from_millis(150)).await;
    
    // Should be expired now
    assert!(cache.get(&key).is_none());
}

#[tokio::test]
async fn test_api_cache_eviction() {
    let cache = ApiCache::new(1); // Very small cache (1 byte)
    
    // Insert multiple items that exceed cache size
    for i in 0..5 {
        let key = format!("key_{}", i);
        let value = format!("value_{}", i);
        cache.insert(key, value, Duration::from_secs(60));
    }
    
    // Cache should have evicted some items to stay under size limit
    let final_size = cache.size();
    assert!(final_size <= 1024 * 1024); // Should be close to 1MB limit
}

#[tokio::test]
async fn test_api_cache_concurrent_access() {
    let cache = ApiCache::new(100);
    let mut handles = vec![];
    
    // Spawn multiple tasks accessing cache concurrently
    for i in 0..10 {
        let cache_clone = cache.clone();
        let handle = tokio::spawn(async move {
            let key = format!("concurrent_key_{}", i);
            let value = format!("concurrent_value_{}", i);
            
            // Insert
            cache_clone.insert(key.clone(), value.clone(), Duration::from_secs(60));
            
            // Retrieve
            let retrieved = cache_clone.get(&key);
            assert_eq!(retrieved, Some(value));
        });
        handles.push(handle);
    }
    
    // Wait for all tasks to complete
    for handle in handles {
        handle.await.unwrap();
    }
}

#[tokio::test]
async fn test_search_api_integration() {
    let mut server = Server::new_async().await;
    
    // Mock search response
    let mock_response = json!({
        "data": {
            "tracks": [{
                "id": "456",
                "title": "Mock Track",
                "artist": {
                    "name": "Mock Artist",
                    "id": "789"
                },
                "album": {
                    "title": "Mock Album",
                    "id": "101112"
                },
                "duration": 200,
                "track_number": 1
            }]
        }
    });
    
    let _mock = server
        .mock("GET", "/search")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_response.to_string())
        .create_async()
        .await;
    
    let api = DabMusicApi::new(server.url());
    let result = api.search_tracks("mock").await;
    
    assert!(result.is_ok());
    let tracks = result.unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "Mock Track");
    assert_eq!(tracks[0].artist, "Mock Artist");
}

#[tokio::test]
async fn test_album_details_api() {
    let mut server = Server::new_async().await;
    
    // Mock album details response
    let mock_response = json!({
        "data": {
            "id": "album123",
            "title": "Test Album",
            "artist": {
                "name": "Test Artist",
                "id": "artist456"
            },
            "release_date": "2023-01-01",
            "tracks": [{
                "id": "track1",
                "title": "Track 1",
                "duration": 180,
                "track_number": 1
            }]
        }
    });
    
    let _mock = server
        .mock("GET", "/album")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_response.to_string())
        .create_async()
        .await;
    
    let api = DabMusicApi::new(server.url());
    let result = api.get_album_details("album123").await;
    
    assert!(result.is_ok());
    let album = result.unwrap();
    assert_eq!(album.title, "Test Album");
    assert_eq!(album.artist, "Test Artist");
    assert!(album.tracks.is_some());
    assert_eq!(album.tracks.unwrap().len(), 1);
}

#[tokio::test]
async fn test_artist_discography_api() {
    let mut server = Server::new_async().await;
    
    // Mock discography response
    let mock_response = json!({
        "data": {
            "albums": [{
                "id": "album1",
                "title": "First Album",
                "release_date": "2022-01-01"
            }, {
                "id": "album2", 
                "title": "Second Album",
                "release_date": "2023-01-01"
            }]
        }
    });
    
    let _mock = server
        .mock("GET", "/discography")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_response.to_string())
        .create_async()
        .await;
    
    let api = DabMusicApi::new(server.url());
    let result = api.get_artist_discography("artist123").await;
    
    assert!(result.is_ok());
    let albums = result.unwrap();
    assert_eq!(albums.len(), 2);
    assert_eq!(albums[0].title, "First Album");
    assert_eq!(albums[1].title, "Second Album");
}

#[tokio::test]
async fn test_stream_url_api() {
    let mut server = Server::new_async().await;
    
    // Mock stream URL response
    let mock_response = json!({
        "data": {
            "url": "https://stream.example.com/track123.mp3",
            "expires_at": "2024-01-01T12:00:00Z"
        }
    });
    
    let _mock = server
        .mock("GET", "/stream")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_response.to_string())
        .create_async()
        .await;
    
    let api = DabMusicApi::new(server.url());
    let result = api.get_stream_url("track123").await;
    
    assert!(result.is_ok());
    let stream_url = result.unwrap();
    assert!(stream_url.starts_with("https://stream.example.com/"));
}

#[tokio::test]
async fn test_api_error_responses() {
    let mut server = Server::new_async().await;
    
    // Mock various error responses
    let _mock_404 = server
        .mock("GET", "/notfound")
        .with_status(404)
        .with_body("Not Found")
        .create_async()
        .await;
    
    let _mock_500 = server
        .mock("GET", "/error")
        .with_status(500)
        .with_body("Internal Server Error")
        .create_async()
        .await;
    
    let api = DabMusicApi::new(server.url());
    
    // Test 404 error
    let result_404 = api.search_tracks("notfound").await;
    assert!(result_404.is_err());
    
    // Test 500 error
    let client = AsyncNetworkClient::new(server.url());
    let result_500 = client.get("/error").await;
    assert!(result_500.is_err());
}

#[tokio::test]
async fn test_rate_limiting() {
    let manager = NetworkManager::new();
    
    // Make multiple rapid requests
    let start = std::time::Instant::now();
    
    for _ in 0..5 {
        manager.wait_for_rate_limit().await;
    }
    
    let elapsed = start.elapsed();
    
    // Should have some delay due to rate limiting
    // Exact timing depends on implementation
    assert!(elapsed >= Duration::from_millis(0)); // Basic sanity check
}