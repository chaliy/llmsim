// Application State Module

use super::config::Config;
use crate::scenario::{AttemptTracker, ScenarioSet};
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
    /// Optional named scenarios, selected per request by a marker in the
    /// user message or a `llmsim-scenario-<name>` model id.
    pub scenarios: Option<Arc<ScenarioSet>>,
    /// Attempt counts per scenario step, for `fail_first` and stream cuts.
    pub attempts: Arc<AttemptTracker>,
}

impl AppState {
    pub fn new(config: Config, stats: SharedStats) -> Self {
        Self {
            config,
            stats,
            script: None,
            scenarios: None,
            attempts: Arc::new(AttemptTracker::default()),
        }
    }

    pub fn with_script(mut self, script: Arc<Script>) -> Self {
        self.script = Some(script);
        self
    }

    pub fn with_scenarios(mut self, scenarios: Arc<ScenarioSet>) -> Self {
        self.scenarios = Some(scenarios);
        self
    }
}
