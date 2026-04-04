# Changelog

All notable changes to ferro-wg will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Log Display Enhancement (Phase 3 US-2)**: Log lines in the TUI now display with timestamps and colored level badges
  - Each log line shows format: `[HH:MM:SS] [LEVEL] message`
  - Level badges use colors: red=ERROR, yellow=WARN, green=INFO, blue=DEBUG
  - Timestamps are cyan-colored for visibility
  - Configurable via `[log_display]` section in config file:
    - `show_timestamps = true/false` (default: true)
    - `color_badges = true/false` (default: true)
  - Backward compatible with legacy log formats
  - Performance optimized with benchmarks included

### Changed
- Added `chrono` dependency for timestamp formatting in daemon logs
- Extended configuration schema to support log display preferences