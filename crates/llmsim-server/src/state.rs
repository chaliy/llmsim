// Application State Module

use crate::config::Config;

/// Shared application state
pub struct AppState {
    pub config: Config,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}
