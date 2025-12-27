//! TUI module for displaying real-time stats dashboard.
//!
//! This module provides a terminal-based dashboard for monitoring
//! LLMSim server statistics in real-time.

mod app;
mod ui;

pub use app::{run_dashboard, DashboardConfig};
