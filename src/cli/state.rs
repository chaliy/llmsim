// Application State Module

use super::config::Config;
use crate::stats::SharedStats;

/// Shared application state
pub struct AppState {
    pub config: Config,
    pub stats: SharedStats,
}

impl AppState {
    pub fn new(config: Config, stats: SharedStats) -> Self {
        Self { config, stats }
    }
}
