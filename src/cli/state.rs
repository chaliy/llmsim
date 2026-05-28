// Application State Module

use super::config::Config;
use crate::script::Script;
use crate::stats::SharedStats;
use std::sync::Arc;

/// Shared application state
pub struct AppState {
    pub config: Config,
    pub stats: SharedStats,
    /// Optional scripted-response source. When set, handlers replay
    /// scripted turns instead of using the configured generator.
    pub script: Option<Arc<Script>>,
}

impl AppState {
    pub fn new(config: Config, stats: SharedStats) -> Self {
        Self {
            config,
            stats,
            script: None,
        }
    }

    pub fn with_script(mut self, script: Arc<Script>) -> Self {
        self.script = Some(script);
        self
    }
}
