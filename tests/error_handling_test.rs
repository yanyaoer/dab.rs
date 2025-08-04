use dab::{DabError, DabResult};
use std::fs;
use std::io;
use tempfile::TempDir;

#[tokio::test]
async fn test_dab_error_creation_and_display() {
    // Test ConfigError
    let config_error = DabError::ConfigError("Invalid configuration value".to_string());
    assert!(config_error.to_string().contains("Invalid configuration value"));
    
    // Test NetworkError
    let network_error = DabError::NetworkError("Connection failed".to_string());
    assert!(network_error.to_string().contains("Connection failed"));
    
    // Test CacheError
    let cache_error = DabError::CacheError("Cache write failed".to_string());
    assert!(cache_error.to_string().contains("Cache write failed"));
    
    // Test PlayerError
    let player_error = DabError::PlayerError("Playback failed".to_string());
    assert!(player_error.to_string().contains("Playback failed"));
    
    // Test ApiError
    let api_error = DabError::ApiError("API request failed".to_string());
    assert!(api_error.to_string().contains("API request failed"));
}

#[tokio::test]
async fn test_dab_error_from_std_errors() {
    // Test conversion from std::io::Error
    let io_error = io::Error::new(io::ErrorKind::NotFound, "File not found");
    let dab_error: DabError = io_error.into();
    
    match dab_error {
        DabError::IoError(_) => {
            // Expected conversion
        }
        _ => panic!("Expected IoError variant"),
    }
    
    // Test conversion from serde_json::Error
    let json_error = serde_json::from_str::<serde_json::Value>("invalid json");
    assert!(json_error.is_err());
    
    let json_error = json_error.unwrap_err();
    let dab_error: DabError = json_error.into();
    
    match dab_error {
        DabError::ParseError(_) => {
            // Expected conversion
        }
        _ => panic!("Expected ParseError variant"),
    }
}

#[tokio::test]
async fn test_dab_result_ok_case() {
    fn successful_operation() -> DabResult<String> {
        Ok("Success".to_string())
    }
    
    let result = successful_operation();
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "Success");
}

#[tokio::test]
async fn test_dab_result_error_case() {
    fn failing_operation() -> DabResult<String> {
        Err(DabError::ConfigError("Operation failed".to_string()))
    }
    
    let result = failing_operation();
    assert!(result.is_err());
    
    if let Err(e) = result {
        assert!(matches!(e, DabError::ConfigError(_)));
    }
}

#[tokio::test]
async fn test_error_chain_propagation() {
    fn inner_operation() -> DabResult<()> {
        Err(DabError::NetworkError("Network timeout".to_string()))
    }
    
    fn outer_operation() -> DabResult<()> {
        inner_operation().map_err(|e| {
            DabError::PlayerError(format!("Player operation failed: {}", e))
        })
    }
    
    let result = outer_operation();
    assert!(result.is_err());
    
    if let Err(e) = result {
        assert!(e.to_string().contains("Player operation failed"));
        assert!(e.to_string().contains("Network timeout"));
    }
}

#[tokio::test]
async fn test_error_context_preservation() {
    fn operation_with_context() -> DabResult<()> {
        let file_result = fs::read_to_string("/nonexistent/path");
        
        match file_result {
            Ok(_) => Ok(()),
            Err(io_error) => {
                // Convert IO error to DabError with additional context
                Err(DabError::IoError(format!(
                    "Failed to read config file: {}",
                    io_error
                )))
            }
        }
    }
    
    let result = operation_with_context();
    assert!(result.is_err());
    
    if let Err(e) = result {
        let error_string = e.to_string();
        assert!(error_string.contains("Failed to read config file"));
    }
}

#[tokio::test]
async fn test_error_recovery_patterns() {
    fn operation_with_fallback() -> DabResult<String> {
        // Try primary operation
        let primary_result: Result<String, DabError> = 
            Err(DabError::NetworkError("Primary service unavailable".to_string()));
        
        // If primary fails, try fallback
        primary_result.or_else(|_| {
            // Fallback operation
            Ok("Fallback result".to_string())
        })
    }
    
    let result = operation_with_fallback();
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "Fallback result");
}

#[tokio::test]
async fn test_error_aggregation() {
    fn multiple_operations() -> DabResult<Vec<String>> {
        let mut results = Vec::new();
        let mut errors = Vec::new();
        
        // Simulate multiple operations, some failing
        let operations = vec![
            Ok("Success 1".to_string()),
            Err(DabError::NetworkError("Error 1".to_string())),
            Ok("Success 2".to_string()),
            Err(DabError::CacheError("Error 2".to_string())),
        ];
        
        for op in operations {
            match op {
                Ok(value) => results.push(value),
                Err(e) => errors.push(e),
            }
        }
        
        if errors.is_empty() {
            Ok(results)
        } else {
            // Aggregate errors into a single error
            let error_messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            Err(DabError::ApiError(format!(
                "Multiple errors occurred: {}",
                error_messages.join(", ")
            )))
        }
    }
    
    let result = multiple_operations();
    assert!(result.is_err());
    
    if let Err(e) = result {
        let error_string = e.to_string();
        assert!(error_string.contains("Multiple errors occurred"));
        assert!(error_string.contains("Error 1"));
        assert!(error_string.contains("Error 2"));
    }
}

#[tokio::test]
async fn test_async_error_handling() {
    async fn async_operation_that_fails() -> DabResult<()> {
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        Err(DabError::PlayerError("Async operation failed".to_string()))
    }
    
    async fn async_operation_with_retry() -> DabResult<()> {
        for attempt in 1..=3 {
            match async_operation_that_fails().await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    if attempt == 3 {
                        return Err(DabError::PlayerError(format!(
                            "Failed after {} attempts: {}",
                            attempt, e
                        )));
                    }
                    // Continue to next attempt
                }
            }
        }
        unreachable!()
    }
    
    let result = async_operation_with_retry().await;
    assert!(result.is_err());
    
    if let Err(e) = result {
        assert!(e.to_string().contains("Failed after 3 attempts"));
    }
}

#[tokio::test]
async fn test_error_logging_integration() {
    fn operation_with_logging() -> DabResult<()> {
        let error = DabError::ConfigError("Test error for logging".to_string());
        
        // In real code, this would use the logging system
        eprintln!("Error occurred: {}", error);
        
        Err(error)
    }
    
    let result = operation_with_logging();
    assert!(result.is_err());
}

#[tokio::test]
async fn test_error_serialization() {
    let error = DabError::NetworkError("Serialization test error".to_string());
    
    // Test Debug formatting
    let debug_string = format!("{:?}", error);
    assert!(debug_string.contains("NetworkError"));
    assert!(debug_string.contains("Serialization test error"));
    
    // Test Display formatting
    let display_string = format!("{}", error);
    assert!(display_string.contains("Serialization test error"));
}

#[tokio::test]
async fn test_custom_error_types() {
    // Test that we can create domain-specific errors
    fn validate_track_id(id: &str) -> DabResult<()> {
        if id.is_empty() {
            return Err(DabError::ApiError("Track ID cannot be empty".to_string()));
        }
        
        if id.len() > 255 {
            return Err(DabError::ApiError("Track ID too long".to_string()));
        }
        
        if !id.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            return Err(DabError::ApiError("Track ID contains invalid characters".to_string()));
        }
        
        Ok(())
    }
    
    // Test valid ID
    assert!(validate_track_id("valid-track_123").is_ok());
    
    // Test invalid IDs
    assert!(validate_track_id("").is_err());
    assert!(validate_track_id(&"x".repeat(300)).is_err());
    assert!(validate_track_id("invalid@id").is_err());
}

#[tokio::test]
async fn test_error_handling_in_concurrent_context() {
    use tokio::sync::mpsc;
    
    async fn worker_task(id: u32, sender: mpsc::UnboundedSender<DabResult<String>>) {
        let result = if id % 2 == 0 {
            Ok(format!("Worker {} completed", id))
        } else {
            Err(DabError::PlayerError(format!("Worker {} failed", id)))
        };
        
        let _ = sender.send(result);
    }
    
    let (sender, mut receiver) = mpsc::unbounded_channel();
    
    // Spawn multiple worker tasks
    for i in 0..4 {
        let sender_clone = sender.clone();
        tokio::spawn(worker_task(i, sender_clone));
    }
    
    drop(sender); // Close the sender
    
    let mut successes = 0;
    let mut failures = 0;
    
    // Collect results
    while let Some(result) = receiver.recv().await {
        match result {
            Ok(_) => successes += 1,
            Err(_) => failures += 1,
        }
    }
    
    assert_eq!(successes, 2); // Workers 0 and 2
    assert_eq!(failures, 2);  // Workers 1 and 3
}

#[tokio::test]
async fn test_error_boundary_pattern() {
    async fn risky_operation(should_fail: bool) -> DabResult<String> {
        if should_fail {
            Err(DabError::NetworkError("Simulated failure".to_string()))
        } else {
            Ok("Success".to_string())
        }
    }
    
    async fn safe_operation_wrapper(should_fail: bool) -> String {
        match risky_operation(should_fail).await {
            Ok(result) => result,
            Err(e) => {
                // Log error and return safe default
                eprintln!("Operation failed: {}", e);
                "Default value".to_string()
            }
        }
    }
    
    // Test success case
    let success_result = safe_operation_wrapper(false).await;
    assert_eq!(success_result, "Success");
    
    // Test failure case with fallback
    let failure_result = safe_operation_wrapper(true).await;
    assert_eq!(failure_result, "Default value");
}

#[tokio::test]
async fn test_error_metrics_collection() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    
    #[derive(Default)]
    struct ErrorMetrics {
        network_errors: AtomicU32,
        cache_errors: AtomicU32,
        player_errors: AtomicU32,
        other_errors: AtomicU32,
    }
    
    impl ErrorMetrics {
        fn record_error(&self, error: &DabError) {
            match error {
                DabError::NetworkError(_) => {
                    self.network_errors.fetch_add(1, Ordering::Relaxed);
                }
                DabError::CacheError(_) => {
                    self.cache_errors.fetch_add(1, Ordering::Relaxed);
                }
                DabError::PlayerError(_) => {
                    self.player_errors.fetch_add(1, Ordering::Relaxed);
                }
                _ => {
                    self.other_errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
    
    let metrics = Arc::new(ErrorMetrics::default());
    
    // Simulate various errors
    let errors = vec![
        DabError::NetworkError("Network error 1".to_string()),
        DabError::NetworkError("Network error 2".to_string()),
        DabError::CacheError("Cache error 1".to_string()),
        DabError::PlayerError("Player error 1".to_string()),
        DabError::ConfigError("Config error 1".to_string()),
    ];
    
    for error in &errors {
        metrics.record_error(error);
    }
    
    // Verify metrics
    assert_eq!(metrics.network_errors.load(Ordering::Relaxed), 2);
    assert_eq!(metrics.cache_errors.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.player_errors.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.other_errors.load(Ordering::Relaxed), 1);
}