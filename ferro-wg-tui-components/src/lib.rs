//! TUI component implementations for ferro-wg.
//!
//! Each module provides a [`Component`](ferro_wg_tui_core::Component)
//! implementation for a specific tab or chrome element:
//!
//! **Tab components:**
//! - [`OverviewComponent`] — aggregate health overview for all connections
//! - [`StatusComponent`] — active tunnel overview (scoped to selected connection)
//! - [`PeersComponent`] — peer configuration details
//! - [`CompareComponent`] — backend performance comparison
//! - [`ConfigComponent`] — interface configuration display
//! - [`LogsComponent`] — live log viewer
//!
//! **Chrome components:**
//! - [`TabBarComponent`] — top-of-screen tab selector
//! - [`StatusBarComponent`] — bottom help text / search input
//! - [`ConnectionBarComponent`] — connection selector strip (multi-connection only)

pub mod compare;
pub mod config;
pub mod confirm_dialog;
pub mod connection_bar;
pub mod logs;
pub mod overview;
pub mod peers;
pub mod status;
pub mod status_bar;
pub mod tab_bar;

pub use compare::CompareComponent;
pub use config::ConfigComponent;
pub use confirm_dialog::ConfirmDialogComponent;
pub use connection_bar::ConnectionBarComponent;
pub use logs::LogsComponent;
pub use overview::OverviewComponent;
pub use peers::PeersComponent;
pub use status::StatusComponent;
pub use status_bar::StatusBarComponent;
pub use tab_bar::TabBarComponent;
