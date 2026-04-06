# Implementation Plan: CLI Logs Command

## Overview
Implement a `logs` command for the ferro-wg CLI that displays daemon logs with filtering capabilities similar to the TUI logs tab. The command should support both streaming (watch mode) and snapshot (tail mode) viewing.

## Requirements
- Filter by log level (trace, debug, info, warn, error) — requires adding #[derive(clap::ValueEnum)] to LogLevel
- Filter by connection name (Option<String>: None = all, Some(name) = specific; global events always pass)
- Search filter (case-insensitive substring match)
- Watch mode: stream new logs continuously with tokio::select! + ctrl_c() for graceful shutdown
- Tail mode: show last N lines (default 50, None = all) via timeout-based drain (200ms timeout, VecDeque<LogEntry> cap at N)
- Output format: timestamp, level, connection, message (plain text; diverges from TUI which omits connection name)

## Implementation Phases

### Phase 3: Filtering Logic (moved first to avoid uncompilable commits)
Extract pure filter functions from ferro-wg-tui-components/src/logs.rs to ferro-wg-core/src/logs.rs:
- `entry_passes_filter()`: applies level, connection, search filters (level >= min_level, trace always passes; connection matches or global; case-insensitive substring)
- `filtered_lines()`: applies filters to batch, returns Vec<LogEntry>
- `line_matches_search()`: case-insensitive substring match
- `level_matches()`: level >= min_level (trace always passes)
- `connection_matches()`: Option<String> filter — None = all, Some(name) = specific, global events always pass

### Phase 1: CLI Definition and Basic Handler
Add `Logs` variant to the `Command` enum in `cli.rs` with options for:
- `--level` (default: debug) — uses LogLevel::ValueEnum
- `--connection` (Option<String>, default: None = all)
- `--search` (default: empty)
- `--lines` (Option<usize>, default: Some(50), None = all)
- `--watch` (flag for streaming mode)

Add `Command::Logs` arm in `main.rs` that calls `cmd_logs()`.
Extract `cmd_logs()` to src/cmd/logs.rs to keep main.rs dispatch-only.

### Phase 2: Core Logs Logic
Implement `cmd_logs()` in src/cmd/logs.rs that:
- Parses CLI args into filter struct using ferro-wg-core::logs functions
- Calls `client::stream_logs()` to get receiver
- In tail mode: timeout drain (tokio::time::timeout 200ms) into VecDeque<LogEntry> capped at N, print and exit
- In watch mode: tokio::select! with receiver loop and ctrl_c() for graceful shutdown
- Applies filters to received entries
- Formats and prints entries using new plain-text formatter (diverges from TUI render_entry)

### Phase 4: Output Formatting
Implement plain text formatting (new formatter, not reusing TUI):
- Timestamp: HH:MM:SS
- Level: padded badge (e.g., [INFO])
- Connection: name or (global) — deliberate divergence from TUI
- Message: full text
- Example: `[14:23:45] [INFO] (global) ferro_wg_core::tunnel: connected`

### Phase 5: Error Handling and Edge Cases
- Handle daemon not running (same as other commands)
- Handle empty log buffer
- Handle invalid filter values
- Graceful shutdown on Ctrl+C in watch mode via tokio::select!
- Memory bounds: VecDeque cap in tail mode

### Phase 6: Testing and Validation
- Unit tests for filter functions (ferro-wg-core)
- Integration tests with mock log entries
- CLI argument parsing tests
- Manual smoke test with running daemon (not automated CI)</content>
<parameter name="filePath">docs/cli-logs-implementation.md