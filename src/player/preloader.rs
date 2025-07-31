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
    user_behavior: Arc<RwLock<UserBehaviorStats>>, // Track user patterns for better prediction
    preload_strategy: Arc<RwLock<PreloadStrategy>>, // Current strategy
}

/// User behavior statistics for smart preloading
#[derive(Debug, Clone)]
pub struct UserBehaviorStats {
    pub skip_rate: f32,                    // Percentage of tracks skipped
    pub average_listen_duration: u32,      // Average listen time in seconds
    pub peak_usage_hours: Vec<u8>,         // Hours when user is most active
    pub preferred_quality: Option<String>, // User's preferred audio quality
    pub recent_skips: u32,                 // Recent skip count (last 10 tracks)
    pub total_tracks_played: u32,          // Total tracks played in session
}

impl Default for UserBehaviorStats {
    fn default() -> Self {
        Self {
            skip_rate: 0.0,
            average_listen_duration: 0,
            peak_usage_hours: Vec::new(),
            preferred_quality: None,
            recent_skips: 0,
            total_tracks_played: 0,
        }
    }
}

/// Enhanced preload strategy
#[derive(Debug)]
pub struct PreloadStrategy {
    pub tracks_to_preload: Vec<PreloadTask>,
    pub priority_levels: Vec<PreloadPriority>,
    pub strategy_type: StrategyType,
    pub confidence_score: f32, // How confident we are in this strategy (0.0-1.0)
}

/// Different preloading strategies based on conditions
#[derive(Debug, Clone)]
pub enum StrategyType {
    Conservative, // Low network/battery - minimal preloading
    Balanced,     // Standard strategy
    Aggressive,   // High network/power - preload more
    Predictive,   // Based on user behavior patterns
    Emergency,    // Network issues detected
}

impl SmartPreloader {
    pub fn new(
        queue: Arc<Queue>,
        loader: Arc<AudioLoader>,
        search_api: Arc<MusicSearchApi>,
    ) -> Self {
        let initial_strategy = PreloadStrategy {
            tracks_to_preload: Vec::new(),
            priority_levels: Vec::new(),
            strategy_type: StrategyType::Balanced,
            confidence_score: 0.5,
        };

        Self {
            queue,
            loader,
            search_api,
            network_speed_kbps: Arc::new(AtomicU32::new(1000)), // Default 1MB/s
            preload_buffer_mb: Arc::new(AtomicU32::new(5)),     // Default 5MB buffer
            preload_ahead_count: 2,                             // Preload next 2 tracks
            last_preload_time: Arc::new(RwLock::new(Instant::now())),
            active_preloads: Arc::new(RwLock::new(std::collections::HashSet::new())),
            user_behavior: Arc::new(RwLock::new(UserBehaviorStats::default())),
            preload_strategy: Arc::new(RwLock::new(initial_strategy)),
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
        let user_behavior = self.user_behavior.clone();
        let preload_strategy = self.preload_strategy.clone();

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
                let user_behavior_guard = user_behavior.read().await;
                let current_strategy_guard = preload_strategy.read().await;
                let strategy = Self::calculate_preload_strategy(
                    &upcoming_tracks,
                    network_speed.load(Ordering::Relaxed),
                    preload_buffer_mb.load(Ordering::Relaxed),
                    &user_behavior_guard,
                    &current_strategy_guard,
                )
                .await;

                // Execute preload strategy
                Self::execute_preload_strategy(
                    strategy,
                    &loader,
                    &search_api,
                    &active_preloads,
                    &last_preload_time,
                )
                .await;
            }
        })
    }

    /// Calculate optimal preload strategy based on network conditions and user behavior
    async fn calculate_preload_strategy(
        upcoming_tracks: &[Track],
        network_speed_kbps: u32,
        buffer_mb: u32,
        user_behavior: &UserBehaviorStats,
        current_strategy: &PreloadStrategy,
    ) -> PreloadStrategy {
        // Determine strategy type based on conditions
        let strategy_type =
            Self::determine_strategy_type(network_speed_kbps, user_behavior, current_strategy);

        let mut new_strategy = PreloadStrategy {
            tracks_to_preload: Vec::new(),
            priority_levels: Vec::new(),
            strategy_type: strategy_type.clone(),
            confidence_score: Self::calculate_confidence_score(network_speed_kbps, user_behavior),
        };

        // Adjust preload count based on strategy and user behavior
        let max_preload_count = match strategy_type {
            StrategyType::Conservative => {
                if user_behavior.skip_rate > 0.7 {
                    1
                } else {
                    2
                }
            }
            StrategyType::Balanced => {
                if user_behavior.skip_rate > 0.5 {
                    2
                } else {
                    3
                }
            }
            StrategyType::Aggressive => {
                if network_speed_kbps > 2000 {
                    5
                } else {
                    4
                }
            }
            StrategyType::Predictive => {
                // Base on user listening patterns
                if user_behavior.average_listen_duration > 180 {
                    4
                } else {
                    2
                }
            }
            StrategyType::Emergency => 1, // Only preload next track
        };

        for (index, track) in upcoming_tracks.iter().enumerate().take(max_preload_count) {
            // Skip local tracks (already available)
            if track.is_local() {
                debug!("Skipping local track for preload: {}", track.title);
                continue;
            }

            // Calculate priority based on multiple factors
            let priority = Self::calculate_track_priority(
                index,
                track,
                &strategy_type,
                user_behavior,
                network_speed_kbps,
            );

            // Skip very low priority tracks in conservative mode
            if matches!(strategy_type, StrategyType::Conservative)
                && matches!(priority, PreloadPriority::Low)
            {
                continue;
            }

            // Calculate dynamic buffer size
            let buffer_size_mb = Self::calculate_dynamic_buffer_size(
                &priority,
                &strategy_type,
                buffer_mb,
                track,
                user_behavior,
            );

            new_strategy.tracks_to_preload.push(PreloadTask {
                track: track.clone(),
                priority,
                buffer_size_mb,
            });
        }

        debug!(
            "Calculated preload strategy: {} tracks, type: {:?}, confidence: {:.2}",
            new_strategy.tracks_to_preload.len(),
            new_strategy.strategy_type,
            new_strategy.confidence_score
        );

        new_strategy
    }

    /// Determine the best strategy type based on current conditions
    fn determine_strategy_type(
        network_speed_kbps: u32,
        user_behavior: &UserBehaviorStats,
        current_strategy: &PreloadStrategy,
    ) -> StrategyType {
        // Check for emergency conditions first
        if network_speed_kbps < 100 {
            return StrategyType::Emergency;
        }

        // High skip rate suggests conservative preloading
        if user_behavior.skip_rate > 0.8 {
            return StrategyType::Conservative;
        }

        // Very fast network with good user engagement -> aggressive
        if network_speed_kbps > 5000 && user_behavior.skip_rate < 0.3 {
            return StrategyType::Aggressive;
        }

        // Use predictive strategy if we have enough user data
        if user_behavior.total_tracks_played > 20 && current_strategy.confidence_score > 0.7 {
            return StrategyType::Predictive;
        }

        // Default to balanced
        StrategyType::Balanced
    }

    /// Calculate priority for a specific track
    fn calculate_track_priority(
        index: usize,
        _track: &Track,
        strategy_type: &StrategyType,
        user_behavior: &UserBehaviorStats,
        network_speed_kbps: u32,
    ) -> PreloadPriority {
        let base_priority = match index {
            0 => PreloadPriority::Critical, // Always critical for next track
            1 => PreloadPriority::High,
            2 => PreloadPriority::Medium,
            _ => PreloadPriority::Low,
        };

        // Adjust based on strategy
        match strategy_type {
            StrategyType::Conservative => {
                // Only upgrade priority for next track
                if index == 0 {
                    base_priority
                } else {
                    PreloadPriority::Low
                }
            }
            StrategyType::Aggressive => {
                // Upgrade priorities if network is good
                if network_speed_kbps > 2000 {
                    match base_priority {
                        PreloadPriority::Low => PreloadPriority::Medium,
                        other => other,
                    }
                } else {
                    base_priority
                }
            }
            StrategyType::Predictive => {
                // Consider user behavior
                if user_behavior.skip_rate < 0.2 && index <= 2 {
                    // User rarely skips, prioritize more tracks
                    match base_priority {
                        PreloadPriority::Medium => PreloadPriority::High,
                        PreloadPriority::Low => PreloadPriority::Medium,
                        other => other,
                    }
                } else {
                    base_priority
                }
            }
            _ => base_priority,
        }
    }

    /// Calculate dynamic buffer size based on multiple factors
    fn calculate_dynamic_buffer_size(
        priority: &PreloadPriority,
        strategy_type: &StrategyType,
        base_buffer_mb: u32,
        track: &Track,
        user_behavior: &UserBehaviorStats,
    ) -> u32 {
        let mut buffer_mb = match priority {
            PreloadPriority::Critical => base_buffer_mb.max(10), // At least 10MB for critical
            PreloadPriority::High => base_buffer_mb.max(5),      // At least 5MB for high
            PreloadPriority::Medium => base_buffer_mb / 2,       // Half buffer for medium
            PreloadPriority::Low => base_buffer_mb / 4,          // Quarter buffer for low
        };

        // Adjust for strategy type
        buffer_mb = match strategy_type {
            StrategyType::Conservative => buffer_mb / 2, // Reduce buffer usage
            StrategyType::Aggressive => buffer_mb * 2,   // Increase buffer usage
            StrategyType::Emergency => buffer_mb.min(2), // Minimal buffer
            _ => buffer_mb,
        };

        // Adjust for track properties
        if track.duration_ms > 0 {
            let duration_seconds = track.duration_ms / 1000;
            if duration_seconds < 60 {
                buffer_mb = buffer_mb / 2; // Smaller buffer for short tracks
            } else if duration_seconds > 600 {
                buffer_mb = buffer_mb * 3 / 2; // Larger buffer for long tracks
            }
        }

        // Consider user listening habits
        if user_behavior.average_listen_duration < 30 {
            // User typically skips quickly, reduce buffer
            buffer_mb = buffer_mb * 2 / 3;
        }

        // Ensure reasonable bounds
        buffer_mb.max(1).min(50) // Between 1MB and 50MB
    }

    /// Calculate confidence score for the strategy
    fn calculate_confidence_score(
        network_speed_kbps: u32,
        user_behavior: &UserBehaviorStats,
    ) -> f32 {
        let mut confidence: f32 = 0.5; // Base confidence

        // Network stability contributes to confidence
        if network_speed_kbps > 1000 {
            confidence += 0.2;
        } else if network_speed_kbps < 500 {
            confidence -= 0.2;
        }

        // User behavior predictability
        if user_behavior.total_tracks_played > 50 {
            confidence += 0.2;
        }

        if user_behavior.skip_rate < 0.3 {
            confidence += 0.1; // Predictable user
        } else if user_behavior.skip_rate > 0.7 {
            confidence -= 0.1; // Unpredictable user
        }

        confidence.max(0.0).min(1.0)
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
                let result =
                    Self::preload_track(track.clone(), loader_clone, search_api_clone).await;

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
            if matches!(
                task.priority,
                PreloadPriority::Critical | PreloadPriority::High
            ) {
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
        let stream_url = match search_api
            .get_stream_url(
                track.track_id.as_ref().ok_or_else(|| {
                    crate::error::DabError::Player("Track missing track_id".to_string())
                })?,
                None,
            )
            .await
        {
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
            debug!(
                "Network speed updated: {} -> {} KB/s",
                old_speed, speed_kbps
            );

            // Adjust buffer size based on network speed
            let new_buffer = if speed_kbps > 2000 {
                10 // 10MB for very fast connections
            } else if speed_kbps > 1000 {
                7 // 7MB for fast connections
            } else if speed_kbps > 500 {
                5 // 5MB for medium connections
            } else {
                3 // 3MB for slow connections
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
                        if let Err(e) = Self::preload_track(track.clone(), loader, search_api).await
                        {
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

    /// Record user behavior event (skip, complete listen, etc.)
    pub async fn record_user_event(&self, event: UserEvent) {
        let mut behavior = self.user_behavior.write().await;

        match event {
            UserEvent::TrackSkipped {
                listen_duration, ..
            } => {
                behavior.recent_skips += 1;
                behavior.total_tracks_played += 1;

                // Update skip rate (rolling average)
                behavior.skip_rate = (behavior.skip_rate * 0.9) + 0.1;

                // Update average listen duration
                let total_duration = behavior.average_listen_duration
                    * (behavior.total_tracks_played - 1)
                    + listen_duration;
                behavior.average_listen_duration = total_duration / behavior.total_tracks_played;
            }
            UserEvent::TrackCompleted { duration, .. } => {
                behavior.total_tracks_played += 1;

                // Update skip rate (track completed, so reduce skip rate)
                behavior.skip_rate = behavior.skip_rate * 0.95;

                // Update average listen duration
                let total_duration = behavior.average_listen_duration
                    * (behavior.total_tracks_played - 1)
                    + duration;
                behavior.average_listen_duration = total_duration / behavior.total_tracks_played;

                // Reset recent skips on completion
                behavior.recent_skips = behavior.recent_skips.saturating_sub(1);
            }
            UserEvent::QualityPreferenceDetected { quality } => {
                behavior.preferred_quality = Some(quality);
            }
        }

        debug!(
            "Updated user behavior: skip_rate={:.2}, avg_duration={}s, total_played={}",
            behavior.skip_rate, behavior.average_listen_duration, behavior.total_tracks_played
        );
    }

    /// Get current user behavior statistics
    pub async fn get_user_behavior(&self) -> UserBehaviorStats {
        self.user_behavior.read().await.clone()
    }

    /// Adaptive strategy adjustment based on recent performance
    pub async fn adjust_strategy_based_on_performance(
        &self,
        performance_metrics: &PerformanceMetrics,
    ) {
        let mut current_strategy = self.preload_strategy.write().await;

        // Reduce confidence if there were many stalls
        if performance_metrics.stall_count > 3 {
            current_strategy.confidence_score *= 0.8;
            warn!(
                "Reducing preload confidence due to {} stalls",
                performance_metrics.stall_count
            );
        }

        // Increase confidence if everything went smoothly
        if performance_metrics.stall_count == 0 && performance_metrics.cache_hit_rate > 0.8 {
            current_strategy.confidence_score = (current_strategy.confidence_score * 1.1).min(1.0);
        }

        // Adjust strategy type if needed
        if performance_metrics.network_issues
            && !matches!(current_strategy.strategy_type, StrategyType::Emergency)
        {
            current_strategy.strategy_type = StrategyType::Conservative;
            info!("Switching to conservative preload strategy due to network issues");
        }
    }
}

/// User behavior events for tracking
#[derive(Debug)]
pub enum UserEvent {
    TrackSkipped {
        track_id: String,
        listen_duration: u32,
    },
    TrackCompleted {
        track_id: String,
        duration: u32,
    },
    QualityPreferenceDetected {
        quality: String,
    },
}

/// Performance metrics for strategy adjustment
#[derive(Debug, Default)]
pub struct PerformanceMetrics {
    pub stall_count: u32,
    pub cache_hit_rate: f32,
    pub network_issues: bool,
    pub average_download_speed: u32,
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
