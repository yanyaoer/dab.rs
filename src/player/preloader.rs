use log::{debug, info, warn};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use super::loader::AudioLoader;
use super::{Queue, Track};
use crate::error::DabResult;
use crate::search::MusicSearchApi;

/// Smart preloader that manages background downloading of upcoming tracks
pub struct SmartPreloader {
    queue: Arc<Queue>,
    loader: Arc<AudioLoader>,
    search_api: Arc<MusicSearchApi>,
    network_speed_kbps: Arc<AtomicU32>, // Network speed in KB/s
    preload_buffer_mb: Arc<AtomicU32>,  // Dynamic buffer size in MB
    preload_ahead_count: usize,         // How many tracks ahead to preload
    last_preload_time: Arc<RwLock<Instant>>,
    active_preloads: Arc<RwLock<std::collections::HashSet<String>>>, // Track IDs being preloaded
}

impl SmartPreloader {
    pub fn new(
        queue: Arc<Queue>,
        loader: Arc<AudioLoader>,
        search_api: Arc<MusicSearchApi>,
    ) -> Self {
        Self {
            queue,
            loader,
            search_api,
            network_speed_kbps: Arc::new(AtomicU32::new(1000)), // Default 1MB/s
            preload_buffer_mb: Arc::new(AtomicU32::new(5)),     // Default 5MB buffer
            preload_ahead_count: 2,                             // Preload next 2 tracks
            last_preload_time: Arc::new(RwLock::new(Instant::now())),
            active_preloads: Arc::new(RwLock::new(std::collections::HashSet::new())),
        }
    }

    /// Start the smart preloader background task
    pub async fn start_background_task(&self) -> tokio::task::JoinHandle<()> {
        let queue = self.queue.clone();
        let loader = self.loader.clone();
        let search_api = self.search_api.clone();
        let network_speed = self.network_speed_kbps.clone();
        let preload_buffer_mb = self.preload_buffer_mb.clone();
        let preload_ahead_count = self.preload_ahead_count;
        let last_preload_time = self.last_preload_time.clone();
        let active_preloads = self.active_preloads.clone();

        tokio::spawn(async move {
            info!("Smart preloader started");
            let mut interval = tokio::time::interval(Duration::from_secs(5)); // Check every 5 seconds

            loop {
                interval.tick().await;

                // Get upcoming tracks from queue
                let upcoming_tracks = match queue.peek_next_n(preload_ahead_count).await {
                    Ok(tracks) => tracks,
                    Err(e) => {
                        warn!("Failed to get upcoming tracks for preload: {}", e);
                        continue;
                    }
                };

                if upcoming_tracks.is_empty() {
                    debug!("No upcoming tracks to preload");
                    continue;
                }

                // Calculate optimal preload strategy
                let strategy = Self::calculate_preload_strategy(
                    &upcoming_tracks,
                    network_speed.load(Ordering::Relaxed),
                    preload_buffer_mb.load(Ordering::Relaxed),
                ).await;

                // Execute preload strategy
                Self::execute_preload_strategy(
                    strategy,
                    &loader,
                    &search_api,
                    &active_preloads,
                    &last_preload_time,
                ).await;
            }
        })
    }

    /// Calculate optimal preload strategy based on network conditions and queue
    async fn calculate_preload_strategy(
        upcoming_tracks: &[Track],
        network_speed_kbps: u32,
        buffer_mb: u32,
    ) -> PreloadStrategy {
        let mut strategy = PreloadStrategy {
            tracks_to_preload: Vec::new(),
            priority_levels: Vec::new(),
        };

        for (index, track) in upcoming_tracks.iter().enumerate() {
            // Skip local tracks (already available)
            if track.is_local() {
                debug!("Skipping local track for preload: {}", track.title);
                continue;
            }

            // Determine priority based on position in queue and network speed
            let priority = if index == 0 {
                // First track gets highest priority
                PreloadPriority::Critical
            } else if index == 1 && network_speed_kbps > 500 {
                // Second track gets high priority if network is good
                PreloadPriority::High
            } else if network_speed_kbps > 1000 {
                // Additional tracks only if network is very good
                PreloadPriority::Medium
            } else {
                PreloadPriority::Low
            };

            // Calculate buffer size based on priority and network speed
            let buffer_size_mb = match priority {
                PreloadPriority::Critical => buffer_mb.max(10), // At least 10MB for critical
                PreloadPriority::High => buffer_mb.max(5),      // At least 5MB for high
                PreloadPriority::Medium => buffer_mb / 2,       // Half buffer for medium
                PreloadPriority::Low => buffer_mb / 4,          // Quarter buffer for low
            };

            strategy.tracks_to_preload.push(PreloadTask {
                track: track.clone(),
                priority,
                buffer_size_mb,
            });

            // Don't preload too many tracks if network is slow
            if network_speed_kbps < 500 && index >= 1 {
                break;
            }
        }

        debug!(
            "Calculated preload strategy: {} tracks, network speed: {} KB/s",
            strategy.tracks_to_preload.len(),
            network_speed_kbps
        );

        strategy
    }

    /// Execute the preload strategy
    async fn execute_preload_strategy(
        strategy: PreloadStrategy,
        loader: &Arc<AudioLoader>,
        search_api: &Arc<MusicSearchApi>,
        active_preloads: &Arc<RwLock<std::collections::HashSet<String>>>,
        last_preload_time: &Arc<RwLock<Instant>>,
    ) {
        let now = Instant::now();
        let last_time = *last_preload_time.read().await;

        // Rate limiting: don't preload too frequently
        if now.duration_since(last_time) < Duration::from_secs(10) {
            debug!("Rate limiting preload execution");
            return;
        }

        for task in strategy.tracks_to_preload {
            // Check if already preloading or preloaded
            {
                let active = active_preloads.read().await;
                if active.contains(&task.track.id) {
                    debug!("Track {} already being preloaded", task.track.id);
                    continue;
                }
            }

            // Check if already cached
            if loader.is_stream_ready(&task.track.id).await {
                debug!("Track {} already cached", task.track.id);
                continue;
            }

            // Add to active preloads
            {
                let mut active = active_preloads.write().await;
                active.insert(task.track.id.clone());
            }

            // Start preload task
            info!(
                "Starting preload for track: {} (priority: {:?})",
                task.track.title, task.priority
            );

            let track = task.track.clone();
            let loader_clone = loader.clone();
            let search_api_clone = search_api.clone();
            let active_preloads_clone = active_preloads.clone();

            tokio::spawn(async move {
                let result = Self::preload_track(track.clone(), loader_clone, search_api_clone).await;

                // Remove from active preloads
                {
                    let mut active = active_preloads_clone.write().await;
                    active.remove(&track.id);
                }

                match result {
                    Ok(()) => {
                        info!("Successfully preloaded track: {}", track.title);
                    }
                    Err(e) => {
                        warn!("Failed to preload track {}: {}", track.title, e);
                    }
                }
            });

            // Don't start too many preloads at once
            if matches!(task.priority, PreloadPriority::Critical | PreloadPriority::High) {
                // Wait a bit between high priority preloads
                tokio::time::sleep(Duration::from_millis(100)).await;
            } else {
                // Wait longer between lower priority preloads
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }

        *last_preload_time.write().await = now;
    }

    /// Preload a single track
    async fn preload_track(
        track: Track,
        loader: Arc<AudioLoader>,
        search_api: Arc<MusicSearchApi>,
    ) -> DabResult<()> {
        // Get stream URL for the track
        let stream_url = match search_api.get_stream_url(
            track.track_id.as_ref().ok_or_else(|| {
                crate::error::DabError::Player("Track missing track_id".to_string())
            })?,
            None,
        ).await {
            Ok(url) => url,
            Err(e) => {
                debug!("Failed to get stream URL for preload: {}", e);
                return Err(e);
            }
        };

        // Start preload with URL
        loader.preload_track_with_url(&track, &stream_url).await
    }

    /// Update network speed measurement (called by download manager)
    pub fn update_network_speed(&self, speed_kbps: u32) {
        let old_speed = self.network_speed_kbps.swap(speed_kbps, Ordering::Relaxed);
        if (speed_kbps as i32 - old_speed as i32).abs() > 100 {
            debug!("Network speed updated: {} -> {} KB/s", old_speed, speed_kbps);
            
            // Adjust buffer size based on network speed
            let new_buffer = if speed_kbps > 2000 {
                10 // 10MB for very fast connections
            } else if speed_kbps > 1000 {
                7  // 7MB for fast connections
            } else if speed_kbps > 500 {
                5  // 5MB for medium connections
            } else {
                3  // 3MB for slow connections
            };
            
            self.preload_buffer_mb.store(new_buffer, Ordering::Relaxed);
        }
    }

    /// Trigger immediate preload for next track (called when user skips)
    pub async fn trigger_immediate_preload(&self) -> DabResult<()> {
        if let Ok(next_tracks) = self.queue.peek_next_n(1).await {
            if let Some(next_track) = next_tracks.first() {
                if !next_track.is_local() && !self.loader.is_stream_ready(&next_track.id).await {
                    info!("Triggering immediate preload for: {}", next_track.title);
                    
                    let track = next_track.clone();
                    let loader = self.loader.clone();
                    let search_api = self.search_api.clone();
                    
                    tokio::spawn(async move {
                        if let Err(e) = Self::preload_track(track.clone(), loader, search_api).await {
                            warn!("Immediate preload failed for {}: {}", track.title, e);
                        }
                    });
                }
            }
        }
        Ok(())
    }

    /// Get preload statistics
    pub async fn get_stats(&self) -> PreloadStats {
        let active_count = self.active_preloads.read().await.len();
        let network_speed = self.network_speed_kbps.load(Ordering::Relaxed);
        let buffer_size = self.preload_buffer_mb.load(Ordering::Relaxed);

        PreloadStats {
            active_preloads: active_count,
            network_speed_kbps: network_speed,
            buffer_size_mb: buffer_size,
        }
    }
}

#[derive(Debug)]
struct PreloadStrategy {
    tracks_to_preload: Vec<PreloadTask>,
    priority_levels: Vec<PreloadPriority>,
}

#[derive(Debug)]
struct PreloadTask {
    track: Track,
    priority: PreloadPriority,
    buffer_size_mb: u32,
}

#[derive(Debug, Clone, Copy)]
enum PreloadPriority {
    Critical, // Next track, must preload
    High,     // Second track, should preload
    Medium,   // Third track, nice to preload
    Low,      // Further tracks, preload if resources available
}

#[derive(Debug)]
pub struct PreloadStats {
    pub active_preloads: usize,
    pub network_speed_kbps: u32,
    pub buffer_size_mb: u32,
}