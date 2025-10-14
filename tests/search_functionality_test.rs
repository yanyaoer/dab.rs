use dab::{search::*, DabError};
use mockito::{Mock, Server};
use serde_json::json;
use std::collections::HashMap;

#[tokio::test]
async fn test_dab_track_creation_and_validation() {
    let track = DabTrack {
        id: "track123".to_string(),
        title: "Test Song".to_string(),
        artist: "Test Artist".to_string(),
        artist_id: Some("artist456".to_string()),
        album: Some("Test Album".to_string()),
        album_id: Some("album789".to_string()),
        duration: Some(180),
        track_number: Some(1),
        disc_number: Some(1),
        year: Some(2023),
        genre: Some("Rock".to_string()),
        explicit: Some(false),
        popularity: Some(85),
        preview_url: Some("https://example.com/preview.mp3".to_string()),
        cover_url: Some("https://example.com/cover.jpg".to_string()),
        streamable: Some(true),
        downloadable: Some(true),
        audio_quality: Some("lossless".to_string()),
    };

    assert_eq!(track.id, "track123");
    assert_eq!(track.title, "Test Song");
    assert_eq!(track.artist, "Test Artist");
    assert_eq!(track.duration, Some(180));

    // Test validation
    assert!(track.is_valid());

    // Test invalid track (empty title)
    let invalid_track = DabTrack {
        title: "".to_string(),
        ..track.clone()
    };
    assert!(!invalid_track.is_valid());
}

#[tokio::test]
async fn test_dab_album_creation_and_validation() {
    let album = DabAlbum {
        id: "album123".to_string(),
        title: "Test Album".to_string(),
        artist: "Test Artist".to_string(),
        artist_id: Some("artist456".to_string()),
        release_date: Some("2023-01-01".to_string()),
        genre: Some("Rock".to_string()),
        cover: Some("https://example.com/cover.jpg".to_string()),
        tracks: Some(vec![DabTrack {
            id: "track1".to_string(),
            title: "Track 1".to_string(),
            artist: "Test Artist".to_string(),
            artist_id: Some("artist456".to_string()),
            album: Some("Test Album".to_string()),
            album_id: Some("album123".to_string()),
            duration: Some(180),
            track_number: Some(1),
            disc_number: Some(1),
            year: Some(2023),
            genre: Some("Rock".to_string()),
            explicit: Some(false),
            popularity: None,
            preview_url: None,
            cover_url: None,
            streamable: Some(true),
            downloadable: Some(true),
            audio_quality: None,
        }]),
        track_count: Some(1),
        duration: Some(180),
        label: Some("Test Label".to_string()),
        upc: Some("123456789012".to_string()),
        url: Some("https://example.com/album".to_string()),
        streamable: Some(true),
        downloadable: Some(true),
        media_count: Some(1),
        maximum_channel_count: Some(2),
        parental_warning: Some(false),
        popularity: Some(80),
        audio_quality: Some("lossless".to_string()),
    };

    assert_eq!(album.id, "album123");
    assert_eq!(album.title, "Test Album");
    assert_eq!(album.artist, "Test Artist");
    assert_eq!(album.track_count, Some(1));
    assert!(album.tracks.is_some());
    assert_eq!(album.tracks.as_ref().unwrap().len(), 1);
}

#[tokio::test]
async fn test_dab_artist_creation_and_validation() {
    let artist = DabArtist {
        id: "artist123".to_string(),
        name: "Test Artist".to_string(),
        picture: Some("https://example.com/artist.jpg".to_string()),
        nb_album: Some(5),
        nb_fan: Some(10000),
        radio: Some(true),
        tracklist: Some("https://api.example.com/artist/123/tracks".to_string()),
        type_field: Some("artist".to_string()),
    };

    assert_eq!(artist.id, "artist123");
    assert_eq!(artist.name, "Test Artist");
    assert_eq!(artist.nb_album, Some(5));
    assert_eq!(artist.nb_fan, Some(10000));
}

#[tokio::test]
async fn test_dab_music_api_initialization() {
    let api = DabMusicApi::new("https://api.example.com".to_string());
    assert_eq!(api.base_url(), "https://api.example.com");
}

#[tokio::test]
async fn test_search_tracks_api() {
    let mut server = Server::new_async().await;

    let mock_response = json!({
        "data": {
            "tracks": [{
                "id": "123",
                "title": "Test Track",
                "artist": {
                    "name": "Test Artist",
                    "id": "456"
                },
                "album": {
                    "title": "Test Album",
                    "id": "789"
                },
                "duration": 200,
                "track_number": 1,
                "explicit": false,
                "preview": "https://example.com/preview.mp3"
            }]
        }
    });

    let _mock = server
        .mock("GET", "/search")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("q".into(), "test query".into()),
            mockito::Matcher::UrlEncoded("type".into(), "track".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_response.to_string())
        .create_async()
        .await;

    let api = DabMusicApi::new(server.url());
    let result = api.search_tracks("test query").await;

    assert!(result.is_ok());
    let tracks = result.unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "Test Track");
    assert_eq!(tracks[0].artist, "Test Artist");
}

#[tokio::test]
async fn test_search_albums_api() {
    let mut server = Server::new_async().await;

    let mock_response = json!({
        "data": {
            "albums": [{
                "id": "album123",
                "title": "Test Album",
                "artist": {
                    "name": "Test Artist",
                    "id": "artist456"
                },
                "release_date": "2023-01-01",
                "nb_tracks": 10,
                "duration": 3600,
                "cover_medium": "https://example.com/cover.jpg"
            }]
        }
    });

    let _mock = server
        .mock("GET", "/search")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("q".into(), "test album".into()),
            mockito::Matcher::UrlEncoded("type".into(), "album".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_response.to_string())
        .create_async()
        .await;

    let api = DabMusicApi::new(server.url());
    let result = api.search_albums("test album").await;

    assert!(result.is_ok());
    let albums = result.unwrap();
    assert_eq!(albums.len(), 1);
    assert_eq!(albums[0].title, "Test Album");
    assert_eq!(albums[0].artist, "Test Artist");
}

#[tokio::test]
async fn test_search_artists_api() {
    let mut server = Server::new_async().await;

    let mock_response = json!({
        "data": {
            "artists": [{
                "id": "artist123",
                "name": "Test Artist",
                "picture_medium": "https://example.com/artist.jpg",
                "nb_album": 5,
                "nb_fan": 10000,
                "radio": true
            }]
        }
    });

    let _mock = server
        .mock("GET", "/search")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("q".into(), "test artist".into()),
            mockito::Matcher::UrlEncoded("type".into(), "artist".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_response.to_string())
        .create_async()
        .await;

    let api = DabMusicApi::new(server.url());
    let result = api.search_artists("test artist").await;

    assert!(result.is_ok());
    let artists = result.unwrap();
    assert_eq!(artists.len(), 1);
    assert_eq!(artists[0].name, "Test Artist");
    assert_eq!(artists[0].nb_album, Some(5));
}

#[tokio::test]
async fn test_get_album_details() {
    let mut server = Server::new_async().await;

    let mock_response = json!({
        "data": {
            "id": "album123",
            "title": "Detailed Album",
            "artist": {
                "name": "Detailed Artist",
                "id": "artist789"
            },
            "release_date": "2023-06-15",
            "tracks": [{
                "id": "track1",
                "title": "Track One",
                "duration": 240,
                "track_position": 1
            }, {
                "id": "track2",
                "title": "Track Two",
                "duration": 180,
                "track_position": 2
            }],
            "nb_tracks": 2,
            "duration": 420,
            "cover_xl": "https://example.com/cover_xl.jpg"
        }
    });

    let _mock = server
        .mock("GET", "/album")
        .match_query(mockito::Matcher::UrlEncoded(
            "albumId".into(),
            "album123".into(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_response.to_string())
        .create_async()
        .await;

    let api = DabMusicApi::new(server.url());
    let result = api.get_album_details("album123").await;

    assert!(result.is_ok());
    let album = result.unwrap();
    assert_eq!(album.title, "Detailed Album");
    assert_eq!(album.artist, "Detailed Artist");
    assert!(album.tracks.is_some());

    let tracks = album.tracks.unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].title, "Track One");
    assert_eq!(tracks[1].title, "Track Two");
}

#[tokio::test]
async fn test_get_artist_discography() {
    let mut server = Server::new_async().await;

    let mock_response = json!({
        "data": {
            "albums": [{
                "id": "album1",
                "title": "First Album",
                "release_date": "2020-01-01",
                "nb_tracks": 12
            }, {
                "id": "album2",
                "title": "Second Album",
                "release_date": "2022-01-01",
                "nb_tracks": 10
            }]
        }
    });

    let _mock = server
        .mock("GET", "/discography")
        .match_query(mockito::Matcher::UrlEncoded(
            "artistId".into(),
            "artist123".into(),
        ))
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
async fn test_get_stream_url() {
    let mut server = Server::new_async().await;

    let mock_response = json!({
        "data": {
            "url": "https://stream.example.com/track123.mp3",
            "expires_at": "2024-01-01T12:00:00Z",
            "format": "mp3",
            "bitrate": 320
        }
    });

    let _mock = server
        .mock("GET", "/stream")
        .match_query(mockito::Matcher::UrlEncoded(
            "trackId".into(),
            "track123".into(),
        ))
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
async fn test_search_error_handling() {
    let mut server = Server::new_async().await;

    // Mock 404 error
    let _mock = server
        .mock("GET", "/search")
        .with_status(404)
        .with_body("Not Found")
        .create_async()
        .await;

    let api = DabMusicApi::new(server.url());
    let result = api.search_tracks("nonexistent").await;

    assert!(result.is_err());
    if let Err(e) = result {
        assert!(matches!(e, DabError::ApiError(_)));
    }
}

#[tokio::test]
async fn test_search_empty_results() {
    let mut server = Server::new_async().await;

    let empty_response = json!({
        "data": {
            "tracks": []
        }
    });

    let _mock = server
        .mock("GET", "/search")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(empty_response.to_string())
        .create_async()
        .await;

    let api = DabMusicApi::new(server.url());
    let result = api.search_tracks("no results").await;

    assert!(result.is_ok());
    let tracks = result.unwrap();
    assert_eq!(tracks.len(), 0);
}

#[tokio::test]
async fn test_search_malformed_response() {
    let mut server = Server::new_async().await;

    let malformed_response = "{ invalid json }";

    let _mock = server
        .mock("GET", "/search")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(malformed_response)
        .create_async()
        .await;

    let api = DabMusicApi::new(server.url());
    let result = api.search_tracks("test").await;

    assert!(result.is_err());
    if let Err(e) = result {
        assert!(matches!(e, DabError::ParseError(_)));
    }
}

#[tokio::test]
async fn test_search_with_special_characters() {
    let mut server = Server::new_async().await;

    let mock_response = json!({
        "data": {
            "tracks": [{
                "id": "special123",
                "title": "Café Müller & Co.",
                "artist": {
                    "name": "Årtist Namé",
                    "id": "artist456"
                },
                "album": {
                    "title": "Álbum Título",
                    "id": "album789"
                },
                "duration": 200
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
    let result = api.search_tracks("Café Müller").await;

    assert!(result.is_ok());
    let tracks = result.unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "Café Müller & Co.");
    assert_eq!(tracks[0].artist, "Årtist Namé");
}

#[tokio::test]
async fn test_search_pagination() {
    let mut server = Server::new_async().await;

    let mock_response = json!({
        "data": {
            "tracks": [
                {
                    "id": "track1",
                    "title": "Track 1",
                    "artist": {"name": "Artist", "id": "artist1"},
                    "duration": 180
                },
                {
                    "id": "track2",
                    "title": "Track 2",
                    "artist": {"name": "Artist", "id": "artist1"},
                    "duration": 200
                }
            ],
            "total": 50,
            "prev": null,
            "next": "https://api.example.com/search?q=test&type=track&index=25"
        }
    });

    let _mock = server
        .mock("GET", "/search")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("q".into(), "test".into()),
            mockito::Matcher::UrlEncoded("type".into(), "track".into()),
            mockito::Matcher::UrlEncoded("limit".into(), "25".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_response.to_string())
        .create_async()
        .await;

    let api = DabMusicApi::new(server.url());
    let result = api.search_tracks_with_limit("test", 25).await;

    assert!(result.is_ok());
    let tracks = result.unwrap();
    assert_eq!(tracks.len(), 2);
}

#[tokio::test]
async fn test_search_filters() {
    let mut server = Server::new_async().await;

    let mock_response = json!({
        "data": {
            "tracks": [{
                "id": "filtered123",
                "title": "Filtered Track",
                "artist": {"name": "Test Artist", "id": "artist456"},
                "album": {"title": "Test Album", "id": "album789"},
                "duration": 180,
                "explicit": false,
                "release_date": "2023-01-01"
            }]
        }
    });

    let _mock = server
        .mock("GET", "/search")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("q".into(), "test".into()),
            mockito::Matcher::UrlEncoded("type".into(), "track".into()),
            mockito::Matcher::UrlEncoded("strict".into(), "on".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_response.to_string())
        .create_async()
        .await;

    let api = DabMusicApi::new(server.url());
    let mut filters = HashMap::new();
    filters.insert("strict".to_string(), "on".to_string());

    let result = api.search_tracks_with_filters("test", filters).await;

    assert!(result.is_ok());
    let tracks = result.unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "Filtered Track");
}

#[tokio::test]
async fn test_concurrent_searches() {
    let mut server = Server::new_async().await;

    let mock_response = json!({
        "data": {
            "tracks": [{
                "id": "concurrent123",
                "title": "Concurrent Track",
                "artist": {"name": "Test Artist", "id": "artist456"},
                "duration": 180
            }]
        }
    });

    let _mock = server
        .mock("GET", "/search")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_response.to_string())
        .expect_at_least(3)
        .create_async()
        .await;

    let api = DabMusicApi::new(server.url());

    // Perform multiple concurrent searches
    let search1 = api.search_tracks("query1");
    let search2 = api.search_tracks("query2");
    let search3 = api.search_tracks("query3");

    let (result1, result2, result3) = tokio::join!(search1, search2, search3);

    assert!(result1.is_ok());
    assert!(result2.is_ok());
    assert!(result3.is_ok());

    assert_eq!(result1.unwrap().len(), 1);
    assert_eq!(result2.unwrap().len(), 1);
    assert_eq!(result3.unwrap().len(), 1);
}
