//! TUI component implementations for ferro-wg.
//!
//! Each module provides a [`Component`](ferro_wg_tui_core::Component)
//! implementation for a specific tab or chrome element:
//!
//! **Tab components:**
//! - [`StatusComponent`] — active tunnel overview
//! - [`PeersComponent`] — peer configuration details
//! - [`CompareComponent`] — backend performance comparison
//! - [`ConfigComponent`] — interface configuration display
//! - [`LogsComponent`] — live log viewer
//!
//! **Chrome components:**
//! - [`TabBarComponent`] — top-of-screen tab selector
//! - [`StatusBarComponent`] — bottom help text / search input

pub mod compare;
pub mod config;
pub mod logs;
pub mod peers;
pub mod status;
pub mod status_bar;
pub mod tab_bar;

pub use compare::CompareComponent;
pub use config::ConfigComponent;
pub use logs::LogsComponent;
pub use peers::PeersComponent;
pub use status::StatusComponent;
pub use status_bar::StatusBarComponent;
pub use tab_bar::TabBarComponent;
