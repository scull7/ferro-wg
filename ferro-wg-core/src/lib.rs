//! `WireGuard` backend abstraction for comparing `boringtun`, `neptun`, and `gotatun`.
//!
//! This crate provides a common [`WgBackend`](backend::WgBackend) trait that
//! normalizes the three userspace `WireGuard` implementations behind a single
//! buffer-oriented API, enabling runtime backend swapping and performance
//! comparison.
//!
//! # Feature Flags
//!
//! Each backend is gated behind its own Cargo feature:
//! - `boringtun` — Cloudflare's implementation
//! - `neptun` — `NordSecurity`'s fork of `boringtun`
//! - `gotatun` — Mullvad's rewrite with owned packet types

pub mod backend;
pub mod config;
pub mod error;
pub mod key;
pub mod stats;
