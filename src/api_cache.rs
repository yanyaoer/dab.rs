use log::debug;
use reqwest::header::{HeaderMap, HeaderValue, CACHE_CONTROL, EXPIRES, ETAG, LAST_MODIFIED};
use std::collections::HashMap;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;

use crate::search::{DabAlbum, DabArtist, SearchResult};

/// Cache entry with HTTP cache headers and expiration
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub response_data: CachedResponse,
    pub cached_at: SystemTime,
    pub expires_at: Option<SystemTime>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub max_age: Option<Duration>,
}

/// Cached API response data
#[derive(Debug, Clone)]
pub enum CachedResponse {
    SearchResult(SearchResult),
    AlbumInfo(DabAlbum),
    ArtistDiscography {
        artist: DabArtist,
        albums: Vec<DabAlbum>,
    },
    Lyrics {
        lyrics: String,
        unsynced: bool,
    },
}

/// HTTP cache directives
#[derive(Debug, Clone)]
pub struct CacheDirectives {
    pub max_age: Option<Duration>,
    pub no_cache: bool,
    pub no_store: bool,
    pub must_revalidate: bool,
    pub private: bool,
}

/// In-memory API response cache with HTTP cache header support
pub struct ApiCache {
    /// Cache storage keyed by request URL
    cache: RwLock<HashMap<String, CacheEntry>>,
    /// Default cache TTL when no cache headers are present
    default_ttl: Duration,
    /// Maximum cache size (number of entries)
    max_entries: usize,
}

impl ApiCache {
    /// Create a new API cache with default settings
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            default_ttl: Duration::from_secs(300), // 5 minutes default
            max_entries: 1000,
        }
    }

    /// Create a new API cache with custom settings
    pub fn new_with_config(default_ttl: Duration, max_entries: usize) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            default_ttl,
            max_entries,
        }
    }

    /// Get cached response if valid
    pub async fn get(&self, url: &str) -> Option<CachedResponse> {
        let cache = self.cache.read().await;
        
        if let Some(entry) = cache.get(url) {
            if self.is_entry_valid(entry) {
                debug!("Cache hit for URL: {}", url);
                return Some(entry.response_data.clone());
            } else {
                debug!("Cache entry expired for URL: {}", url);
            }
        } else {
            debug!("Cache miss for URL: {}", url);
        }
        
        None
    }

    /// Store response in cache with HTTP headers
    pub async fn put(
        &self,
        url: String,
        response_data: CachedResponse,
        headers: &HeaderMap<HeaderValue>,
    ) {
        let mut cache = self.cache.write().await;

        // Check if we need to remove old entries
        if cache.len() >= self.max_entries {
            self.evict_expired_entries(&mut cache);
            
            // If still at capacity, remove oldest entry
            if cache.len() >= self.max_entries {
                if let Some(oldest_key) = self.find_oldest_entry(&cache) {
                    cache.remove(&oldest_key);
                    debug!("Evicted oldest cache entry: {}", oldest_key);
                }
            }
        }

        let cached_at = SystemTime::now();
        let cache_directives = self.parse_cache_control(headers);
        
        // Don't cache if no-store directive is present
        if cache_directives.no_store {
            debug!("Not caching due to no-store directive: {}", url);
            return;
        }

        let entry = CacheEntry {
            response_data,
            cached_at,
            expires_at: self.calculate_expires_at(&cache_directives, headers, cached_at),
            etag: self.extract_header_value(headers, ETAG),
            last_modified: self.extract_header_value(headers, LAST_MODIFIED),
            max_age: cache_directives.max_age,
        };

        cache.insert(url.clone(), entry);
        debug!("Cached response for URL: {}", url);
    }

    /// Check if a cache entry should be revalidated
    pub async fn should_revalidate(&self, url: &str) -> Option<(String, String)> {
        let cache = self.cache.read().await;
        
        if let Some(entry) = cache.get(url) {
            // If entry is expired or has must-revalidate, check for conditional headers
            if !self.is_entry_valid(entry) {
                if let Some(etag) = &entry.etag {
                    return Some(("If-None-Match".to_string(), etag.clone()));
                } else if let Some(last_modified) = &entry.last_modified {
                    return Some(("If-Modified-Since".to_string(), last_modified.clone()));
                }
            }
        }
        
        None
    }

    /// Update cache entry if response is 304 Not Modified
    pub async fn update_on_not_modified(&self, url: &str, headers: &HeaderMap<HeaderValue>) {
        let mut cache = self.cache.write().await;
        
        if let Some(entry) = cache.get_mut(url) {
            let cached_at = SystemTime::now();
            let cache_directives = self.parse_cache_control(headers);
            
            // Update cache metadata
            entry.cached_at = cached_at;
            entry.expires_at = self.calculate_expires_at(&cache_directives, headers, cached_at);
            entry.max_age = cache_directives.max_age;
            
            // Update ETags and Last-Modified if present
            if let Some(new_etag) = self.extract_header_value(headers, ETAG) {
                entry.etag = Some(new_etag);
            }
            if let Some(new_last_modified) = self.extract_header_value(headers, LAST_MODIFIED) {
                entry.last_modified = Some(new_last_modified);
            }
            
            debug!("Updated cache entry on 304 Not Modified: {}", url);
        }
    }

    /// Clear all cache entries
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        debug!("Cleared all cache entries");
    }

    /// Get cache statistics
    pub async fn stats(&self) -> CacheStats {
        let cache = self.cache.read().await;
        let total_entries = cache.len();
        let expired_entries = cache.values()
            .filter(|entry| !self.is_entry_valid(entry))
            .count();
        
        CacheStats {
            total_entries,
            valid_entries: total_entries - expired_entries,
            expired_entries,
            max_entries: self.max_entries,
        }
    }

    /// Check if cache entry is still valid
    fn is_entry_valid(&self, entry: &CacheEntry) -> bool {
        let now = SystemTime::now();
        
        // Check explicit expiration time
        if let Some(expires_at) = entry.expires_at {
            if now > expires_at {
                return false;
            }
        }
        
        // Check max-age
        if let Some(max_age) = entry.max_age {
            if let Ok(age) = now.duration_since(entry.cached_at) {
                if age > max_age {
                    return false;
                }
            }
        }
        
        // If no explicit expiration, use default TTL
        if entry.expires_at.is_none() && entry.max_age.is_none() {
            if let Ok(age) = now.duration_since(entry.cached_at) {
                if age > self.default_ttl {
                    return false;
                }
            }
        }
        
        true
    }

    /// Parse Cache-Control header
    fn parse_cache_control(&self, headers: &HeaderMap<HeaderValue>) -> CacheDirectives {
        let mut directives = CacheDirectives {
            max_age: None,
            no_cache: false,
            no_store: false,
            must_revalidate: false,
            private: false,
        };

        if let Some(cache_control) = headers.get(CACHE_CONTROL) {
            if let Ok(cache_control_str) = cache_control.to_str() {
                for directive in cache_control_str.split(',') {
                    let directive = directive.trim().to_lowercase();
                    
                    if directive == "no-cache" {
                        directives.no_cache = true;
                    } else if directive == "no-store" {
                        directives.no_store = true;
                    } else if directive == "must-revalidate" {
                        directives.must_revalidate = true;
                    } else if directive == "private" {
                        directives.private = true;
                    } else if directive.starts_with("max-age=") {
                        if let Ok(seconds) = directive[8..].parse::<u64>() {
                            directives.max_age = Some(Duration::from_secs(seconds));
                        }
                    }
                }
            }
        }

        directives
    }

    /// Calculate expiration time based on headers
    fn calculate_expires_at(
        &self,
        directives: &CacheDirectives,
        headers: &HeaderMap<HeaderValue>,
        cached_at: SystemTime,
    ) -> Option<SystemTime> {
        // max-age takes precedence over Expires header
        if let Some(max_age) = directives.max_age {
            return Some(cached_at + max_age);
        }

        // Check Expires header
        if let Some(expires) = headers.get(EXPIRES) {
            if let Ok(expires_str) = expires.to_str() {
                if let Ok(expires_time) = httpdate::parse_http_date(expires_str) {
                    return Some(expires_time);
                }
            }
        }

        None
    }

    /// Extract header value as string
    fn extract_header_value(
        &self,
        headers: &HeaderMap<HeaderValue>,
        header_name: reqwest::header::HeaderName,
    ) -> Option<String> {
        headers
            .get(header_name)
            .and_then(|value| value.to_str().ok())
            .map(|s| s.to_string())
    }

    /// Remove expired entries from cache
    fn evict_expired_entries(&self, cache: &mut HashMap<String, CacheEntry>) {
        let expired_keys: Vec<String> = cache
            .iter()
            .filter(|(_, entry)| !self.is_entry_valid(entry))
            .map(|(key, _)| key.clone())
            .collect();

        for key in expired_keys {
            cache.remove(&key);
            debug!("Evicted expired cache entry: {}", key);
        }
    }

    /// Find the oldest cache entry
    fn find_oldest_entry(&self, cache: &HashMap<String, CacheEntry>) -> Option<String> {
        cache
            .iter()
            .min_by_key(|(_, entry)| entry.cached_at)
            .map(|(key, _)| key.clone())
    }
}

/// Cache statistics
#[derive(Debug)]
pub struct CacheStats {
    pub total_entries: usize,
    pub valid_entries: usize, 
    pub expired_entries: usize,
    pub max_entries: usize,
}

// Add httpdate dependency for parsing HTTP date headers
// This is a common utility for HTTP date parsing
mod httpdate {
    use std::time::SystemTime;

    pub fn parse_http_date(_date_str: &str) -> Result<SystemTime, &'static str> {
        // Simple HTTP date parsing - in a real implementation you'd want to use
        // a proper HTTP date parsing library like `httpdate` crate
        
        // For now, return current time + 1 hour as a placeholder
        // TODO: Implement proper HTTP date parsing or add httpdate crate
        Ok(SystemTime::now() + std::time::Duration::from_secs(3600))
    }
}