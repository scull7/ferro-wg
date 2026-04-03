//! Core types for the ferro-wg TUI.
//!
//! This crate provides the foundational abstractions shared across the
//! TUI crate family:
//!
//! - [`Action`] — central enum for unidirectional state changes
//! - [`AppState`] — centralized application state with dispatch
//! - [`Component`] — trait that every TUI panel implements
//! - [`Theme`] — semantic color palette (Catppuccin Mocha / Latte)
//! - [`Tab`] / [`InputMode`] — navigation and input-mode enums

pub mod action;
pub mod app;
pub mod component;
pub mod state;
pub mod theme;
pub mod util;

pub use action::Action;
pub use app::{InputMode, Tab};
pub use component::Component;
pub use state::{AppState, ConnectionState, ConnectionStatus, ConnectionView, Feedback};
pub use theme::Theme;
pub use util::format_bytes;
