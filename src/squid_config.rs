use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::RwLock;

/// Squid API service configuration
/// Based on tidal-ui/src/lib/config.ts architecture
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SquidServiceConfig {
    /// List of available Squid API endpoints
    pub targets: Vec<SquidTarget>,

    /// Primary/default endpoint
    pub primary_target: String,

    /// Enable weighted random selection
    pub use_load_balancing: bool,

    /// Enable automatic failover on error
    pub enable_failover: bool,

    /// Proxy configuration for CORS issues
    pub proxy: ProxyConfig,

    /// Request timeout in seconds
    pub timeout_seconds: u64,

    /// Max retry attempts per target
    pub max_retries: u32,

    /// Delay between retries in milliseconds
    pub retry_delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SquidTarget {
    /// Unique name for this target
    pub name: String,

    /// Base URL for the API endpoint
    pub base_url: String,

    /// Weight for load balancing (higher = more requests)
    pub weight: u32,

    /// Whether this endpoint requires proxy
    pub requires_proxy: bool,

    /// Is this endpoint currently healthy
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_healthy: Option<bool>,

    /// Average response time in ms
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_response_time: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// Enable proxy for CORS bypass
    pub enabled: bool,

    /// Proxy server URL
    pub url: Option<String>,
}

impl Default for SquidServiceConfig {
    fn default() -> Self {
        Self {
            targets: vec![
                SquidTarget {
                    name: "kraken-primary".to_string(),
                    base_url: "https://kraken.squid.wtf".to_string(),
                    weight: 25,
                    requires_proxy: false,
                    is_healthy: Some(true),
                    avg_response_time: None,
                },
                SquidTarget {
                    name: "triton-secondary".to_string(),
                    base_url: "https://triton.squid.wtf".to_string(),
                    weight: 25,
                    requires_proxy: false,
                    is_healthy: Some(true),
                    avg_response_time: None,
                },
                SquidTarget {
                    name: "zeus-tertiary".to_string(),
                    base_url: "https://zeus.squid.wtf".to_string(),
                    weight: 20,
                    requires_proxy: false,
                    is_healthy: Some(true),
                    avg_response_time: None,
                },
                SquidTarget {
                    name: "aether-quaternary".to_string(),
                    base_url: "https://aether.squid.wtf".to_string(),
                    weight: 20,
                    requires_proxy: false,
                    is_healthy: Some(true),
                    avg_response_time: None,
                },
                SquidTarget {
                    name: "vercel-fastapi".to_string(),
                    base_url: "https://tidal-api-2.binimum.org".to_string(),
                    weight: 5,
                    requires_proxy: false,
                    is_healthy: Some(true),
                    avg_response_time: None,
                },
                SquidTarget {
                    name: "proxied-primary".to_string(),
                    base_url: "https://tidal.401658.xyz".to_string(),
                    weight: 5,
                    requires_proxy: false,
                    is_healthy: Some(true),
                    avg_response_time: None,
                },
            ],
            primary_target: "kraken-primary".to_string(),
            use_load_balancing: true,
            enable_failover: true,
            proxy: ProxyConfig {
                enabled: false,
                url: None,
            },
            timeout_seconds: 30,
            max_retries: 3,
            retry_delay_ms: 500,
        }
    }
}

/// Weighted target with cumulative weight for selection
#[derive(Debug, Clone)]
struct WeightedTarget {
    target: SquidTarget,
    cumulative_weight: u32,
}

/// Squid API endpoint selector with load balancing and failover
pub struct SquidTargetSelector {
    config: Arc<RwLock<SquidServiceConfig>>,
    weighted_targets: Arc<RwLock<Vec<WeightedTarget>>>,
}

impl SquidTargetSelector {
    pub fn new(config: SquidServiceConfig) -> Self {
        let weighted = Self::build_weighted_targets(&config);
        Self {
            config: Arc::new(RwLock::new(config)),
            weighted_targets: Arc::new(RwLock::new(weighted)),
        }
    }

    /// Build weighted targets for random selection
    fn build_weighted_targets(config: &SquidServiceConfig) -> Vec<WeightedTarget> {
        let mut cumulative = 0u32;
        let mut weighted_targets = Vec::new();

        for target in &config.targets {
            // Only include healthy targets
            if target.is_healthy.unwrap_or(true) && target.weight > 0 {
                cumulative += target.weight;
                weighted_targets.push(WeightedTarget {
                    target: target.clone(),
                    cumulative_weight: cumulative,
                });
            }
        }

        weighted_targets
    }

    /// Select a target based on weight distribution
    pub fn select_target(&self) -> Option<SquidTarget> {
        let config = self.config.read().unwrap();

        // If load balancing is disabled, return primary target
        if !config.use_load_balancing {
            return config.targets
                .iter()
                .find(|t| t.name == config.primary_target)
                .cloned();
        }

        // Use weighted random selection
        let weighted = self.weighted_targets.read().unwrap();
        if weighted.is_empty() {
            return None;
        }

        let total_weight = weighted.last()?.cumulative_weight;
        let random = (rand::random::<f32>() * total_weight as f32) as u32;

        for wt in weighted.iter() {
            if random < wt.cumulative_weight {
                return Some(wt.target.clone());
            }
        }

        // Fallback to first available
        Some(weighted[0].target.clone())
    }

    /// Get primary target
    pub fn get_primary_target(&self) -> Option<SquidTarget> {
        let config = self.config.read().unwrap();
        config.targets
            .iter()
            .find(|t| t.name == config.primary_target)
            .cloned()
    }

    /// Get all targets ordered by weight (descending)
    pub fn get_targets_by_weight(&self) -> Vec<SquidTarget> {
        let config = self.config.read().unwrap();
        let mut targets = config.targets.clone();
        targets.sort_by(|a, b| b.weight.cmp(&a.weight));
        targets
    }

    /// Get all healthy targets for failover attempts
    pub fn get_failover_targets(&self) -> Vec<SquidTarget> {
        let config = self.config.read().unwrap();

        if !config.enable_failover {
            // If failover is disabled, only return primary
            return self.get_primary_target().into_iter().collect();
        }

        // Return all healthy targets ordered by weight
        config.targets
            .iter()
            .filter(|t| t.is_healthy.unwrap_or(true))
            .cloned()
            .collect()
    }

    /// Mark a target as unhealthy
    pub fn mark_unhealthy(&self, target_name: &str) {
        let mut config = self.config.write().unwrap();
        if let Some(target) = config.targets.iter_mut().find(|t| t.name == target_name) {
            target.is_healthy = Some(false);
        }

        // Rebuild weighted targets
        let weighted = Self::build_weighted_targets(&config);
        *self.weighted_targets.write().unwrap() = weighted;
    }

    /// Mark a target as healthy
    pub fn mark_healthy(&self, target_name: &str) {
        let mut config = self.config.write().unwrap();
        if let Some(target) = config.targets.iter_mut().find(|t| t.name == target_name) {
            target.is_healthy = Some(true);
        }

        // Rebuild weighted targets
        let weighted = Self::build_weighted_targets(&config);
        *self.weighted_targets.write().unwrap() = weighted;
    }

    /// Update average response time for a target
    pub fn update_response_time(&self, target_name: &str, response_time_ms: u64) {
        let mut config = self.config.write().unwrap();
        if let Some(target) = config.targets.iter_mut().find(|t| t.name == target_name) {
            // Simple moving average
            target.avg_response_time = Some(match target.avg_response_time {
                Some(avg) => (avg * 4 + response_time_ms) / 5, // Weight recent: 20%
                None => response_time_ms,
            });
        }
    }

    /// Determine if a path should prefer specific targets
    pub fn should_prefer_primary(path: &str) -> bool {
        let path_lower = path.to_lowercase();

        // Prefer primary for these endpoints (similar to tidal-ui logic)
        path_lower.contains("/album/") ||
        path_lower.contains("/artist/") ||
        path_lower.contains("/playlist/") ||
        (path_lower.contains("/search/") &&
         (path_lower.contains("?a") || path_lower.contains("?al") || path_lower.contains("?p")))
    }

    /// Get proxy URL if needed for a target
    pub fn get_proxy_url(&self, target: &SquidTarget, original_url: &str) -> String {
        let config = self.config.read().unwrap();

        if target.requires_proxy && config.proxy.enabled {
            if let Some(ref proxy_url) = config.proxy.url {
                return format!("{}?url={}", proxy_url, urlencoding::encode(original_url));
            }
        }

        original_url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = SquidServiceConfig::default();
        assert_eq!(config.targets.len(), 6);
        assert_eq!(config.primary_target, "kraken-primary");
        assert!(config.use_load_balancing);
        assert!(config.enable_failover);
    }

    #[test]
    fn test_target_selection() {
        let config = SquidServiceConfig::default();
        let selector = SquidTargetSelector::new(config);

        // Should return a target
        let target = selector.select_target();
        assert!(target.is_some());

        // Primary target should be findable
        let primary = selector.get_primary_target();
        assert!(primary.is_some());
        assert_eq!(primary.unwrap().name, "kraken-primary");
    }

    #[test]
    fn test_health_marking() {
        let config = SquidServiceConfig::default();
        let selector = SquidTargetSelector::new(config);

        // Mark a target unhealthy
        selector.mark_unhealthy("kraken-primary");

        // Should not select unhealthy targets frequently
        let mut selections = Vec::new();
        for _ in 0..100 {
            if let Some(target) = selector.select_target() {
                selections.push(target.name);
            }
        }

        // Unhealthy target should not appear
        let unhealthy_count = selections.iter()
            .filter(|&name| name == "kraken-primary")
            .count();
        assert_eq!(unhealthy_count, 0);
    }

    #[test]
    fn test_failover_targets() {
        let mut config = SquidServiceConfig::default();
        config.enable_failover = true;
        let selector = SquidTargetSelector::new(config);

        let targets = selector.get_failover_targets();
        assert_eq!(targets.len(), 6); // All targets should be available

        // Mark one unhealthy
        selector.mark_unhealthy("kraken-primary");
        let targets = selector.get_failover_targets();
        assert_eq!(targets.len(), 5); // One less target
    }
}