# Phase 3: Log Streaming — Implementation Plan

## Pre-implementation checklist

Before writing any Rust code, invoke `/rust-practices` (per `CLAUDE.md`).
Also confirm `tokio::sync::broadcast` is already in `ferro-wg-core`'s
`[dependencies]` before reaching for it — it ships as part of `tokio` with the
`sync` feature and should already be enabled, but verify before adding a
redundant entry.

---

## Context

Phase 2 delivered multi-connection support: all configured connections are
visible, individually selectable, and independently live-polled. The Logs tab
exists but is a placeholder — it renders `state.log_lines: Vec<String>` which
is never populated and `LogsComponent` is a stub. **Neither `LogEntry`,
`LogBroadcaster`, nor any log-streaming IPC plumbing exists yet; this document
is the spec, not a description of implemented code.**

Phase 3 makes logs live. The daemon already uses `tracing` for internal
diagnostics; this phase captures those events via a new custom tracing `Layer`,
broadcasts them over a dedicated long-lived IPC connection, and displays them
in a fully interactive `LogsComponent` with level filtering, per-connection
filtering, scroll, and in-viewer search.

**Done when:** logs stream in real time with < 100 ms latency from daemon event
to TUI display; level filtering and per-connection filtering work without
dropping messages; the buffer wraps cleanly at capacity without stutter; the
stream reconnects automatically when the daemon restarts.

---

## User Stories

| ID | User story | Acceptance criteria | Step |
|----|------------|---------------------|------|
| US-1 | As a user I want to see daemon log output in the Logs tab in real time | New entries appear within 100 ms of the daemon emitting them; the tab shows live updates without any manual refresh | 4 |
| US-2 | As a user I want each log line to show a timestamp and a level badge | Each line renders `HH:MM:SS  LEVEL  message`; badge colour matches level (red=ERROR, yellow=WARN, green=INFO, blue=DEBUG) | 5 |
| US-3 | As a user I want to scroll through the log history with keyboard shortcuts | `↑`/`k` scrolls toward older entries; `↓`/`j` toward newer; `g` jumps to oldest; `G` jumps to newest and re-enables auto-scroll | 5 |
| US-4 | As a user I want to filter logs by minimum severity | Pressing `l` cycles the minimum level DEBUG → INFO → WARN → ERROR → DEBUG; entries below the threshold are hidden without being discarded from the buffer | 5 |
| US-5 | As a user I want to search within visible log lines | Pressing `/` enters search mode; matching lines are highlighted; `Esc` exits search without clearing results | 5 |
| US-6 | As a user I want to filter logs to the currently selected connection | Pressing `c` toggles between "all connections" and "current connection only"; the block title reflects the active filter | 5 |
| US-7 | As a user I want the view to auto-scroll when I'm reading the newest entries | When `scroll_offset == 0` (pinned to bottom), new entries scroll into view automatically; scrolling up freezes the view | 5 |
| US-8 | As a user I want the buffer to wrap cleanly when it fills up | When the buffer reaches its capacity (default 1 000 entries), the oldest entry is evicted; no visible freeze or stutter | 3 |
| US-9 | As a user I want the Logs tab to recover if the daemon restarts | The stream task reconnects automatically after a 2 s backoff; a "reconnecting…" indicator appears in the status bar during the gap | 4 |
| US-10 | As a daemon operator I want log capture to impose no performance penalty | The tracing `Layer` sends to a bounded `broadcast` channel (capacity 512); lagging subscribers are dropped with `RecvError::Lagged`, not blocking the daemon | 2 |

---

## Architecture

```
ferro-wg-core
  ├── ipc.rs          ← LogLevel, LogEntry, DaemonCommand::StreamLogs,
  │                      DaemonResponse::LogEntry
  └── log_stream.rs   ← LogBroadcaster (broadcast::Sender + history ringbuffer)
                         TuiTracingLayer (tracing::Layer impl)

ferro-wg-daemon / ferro-wg daemon subcommand
  └── main.rs         ← create LogBroadcaster, install TuiTracingLayer, pass to daemon::run()

ferro-wg-core / daemon.rs
  └── run()           ← tokio::spawn streaming handler for StreamLogs command;
                         existing one-shot path unchanged

ferro-wg-tui-core
  ├── action.rs       ← AppendLog, SetLogMinLevel, LogStreamConnected/Disconnected
  └── state.rs        ← log_buffer: LogBuffer (replaces log_lines: Vec<String>)

ferro-wg-tui
  └── lib.rs          ← spawn_log_stream() task, DaemonMessage::LogEntry / LogStreamDisconnected

ferro-wg-tui-components
  └── logs.rs         ← full LogsComponent with scroll, level filter, connection filter, search
```

### Data flow

```
tracing macro (daemon)
    │
    ▼
TuiTracingLayer::on_event()
    │ broadcast::send(LogEntry)
    ▼
LogBroadcaster (broadcast::Sender<LogEntry> + VecDeque history)
    │
    ├─ history: replayed to new subscribers on connect
    │
    └─ broadcast::Receiver<LogEntry>  ←  streaming handler (tokio::spawn per client)
           │ DaemonResponse::LogEntry (newline-delimited JSON over Unix socket)
           ▼
    TUI log stream task (ferro-wg-tui)
           │ DaemonMessage::LogEntry(LogEntry)
           ▼
    event_loop drain → Action::AppendLog(LogEntry)
           │
           ▼
    AppState::dispatch → log_buffer.push(entry)
           │
           ▼
    LogsComponent::render → filtered, colourized, scrollable view
```

### Long-lived vs one-shot IPC connections

The existing daemon server loop accepts a connection, reads **one** command line,
sends one response, and closes. `StreamLogs` cannot fit that model.

On detecting `DaemonCommand::StreamLogs`, the server loop `tokio::spawn`s a
dedicated streaming task and loops back to `accept()` immediately. The spawned
task holds the socket open, replays history, then forwards broadcast events
until the writer fails (client disconnected). This keeps the existing one-shot
path entirely unchanged.

### Broadcast channel and backpressure

`broadcast::Sender<LogEntry>` has a fixed capacity (512 entries). If a TUI
client reads too slowly, `recv()` returns `RecvError::Lagged(n)` — the client
skips `n` entries and appends a synthetic `"[n log entries dropped]"` warning
entry to `log_buffer`. The daemon is never blocked.

### History replay

`LogBroadcaster` maintains a `VecDeque<LogEntry>` of the last 200 entries.
When a new subscriber connects, the streaming handler atomically snapshots
history and sends it before forwarding live events, so the Logs tab is not
empty on first open.

---

## Dependency Graph

```
ferro-wg-core          ← Step 1: ipc.rs  (LogLevel, LogEntry, new IPC variants)
ferro-wg-core          ← Step 2: log_stream.rs  (LogBroadcaster, TuiTracingLayer)
ferro-wg-core/daemon.rs ← Step 2: spawn streaming handler for StreamLogs
ferro-wg-daemon / ferro-wg ← Step 2: wire broadcaster at startup
    ↑
ferro-wg-tui-core      ← Step 3: LogBuffer, AppState, new Actions
    ↑
ferro-wg-tui           ← Step 4: spawn_log_stream(), DaemonMessage variants
    ↑
ferro-wg-tui-components ← Step 5: LogsComponent full implementation
```

---

## Implementation Steps

### Step 1 — IPC protocol extension (`ferro-wg-core/src/ipc.rs`)

**New types:**

```rust
/// Severity level for a daemon log entry.
///
/// Ordered from most to least severe so comparisons like `entry.level >= min_level`
/// work correctly with `PartialOrd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LogLevel {
    Error = 4,
    Warn  = 3,
    Info  = 2,
    Debug = 1,
    Trace = 0,
}

impl LogLevel {
    /// Human-readable short label used in the TUI badge.
    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn  => "WARN ",
            Self::Info  => "INFO ",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }

    /// Advance to the next stricter minimum severity for UI filtering
    /// (DEBUG → INFO → WARN → ERROR → DEBUG, wrapping).
    /// "Next filter level" means fewer entries will be shown after the call.
    pub fn next_filter_level(self) -> Self {
        match self {
            Self::Trace => Self::Debug,
            Self::Debug => Self::Info,
            Self::Info  => Self::Warn,
            Self::Warn  => Self::Error,
            Self::Error => Self::Trace,
        }
    }
}

/// A single structured log event emitted by the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Milliseconds since UNIX epoch, **always UTC**.
    /// Chosen over `DateTime` to avoid a `chrono` dependency on the IPC
    /// boundary. The daemon records wall-clock UTC via
    /// `SystemTime::now().duration_since(UNIX_EPOCH)`.
    /// The TUI converts to local time for display using the host's timezone.
    pub timestamp_ms: i64,
    /// Severity level.
    pub level: LogLevel,
    /// Connection name this event is associated with, if any.
    /// `None` means a global daemon event not tied to a specific tunnel.
    pub connection_name: Option<String>,
    /// The `tracing` target (usually the Rust module path).
    pub target: String,
    /// Formatted log message.
    pub message: String,
}
```

**New `DaemonCommand` variant:**

```rust
/// Open a persistent log stream. The daemon sends `DaemonResponse::LogEntry`
/// messages on this connection until the client disconnects.
/// Recent history is replayed first (up to 200 entries).
StreamLogs {
    /// Filter to a specific connection. `None` = receive logs for all
    /// connections and global daemon events.
    connection_name: Option<String>,
    /// Minimum level to forward; entries below this level are suppressed
    /// server-side before transmission.
    min_level: LogLevel,
},
```

**New `DaemonResponse` variant:**

```rust
/// A single log entry pushed from an active `StreamLogs` subscription.
LogEntry(LogEntry),
```

#### Tests

| Test | Assertion |
|------|-----------|
| `log_level_ordering` | `Error > Warn > Info > Debug > Trace` |
| `log_level_next_filter_level_wraps` | `Error.next_filter_level() == Trace` |
| `log_entry_roundtrip` | Serialize → deserialize preserves all fields |
| `stream_logs_command_roundtrip` | `StreamLogs { connection_name: Some("mia"), min_level: Info }` survives encode/decode |
| `log_entry_response_roundtrip` | `DaemonResponse::LogEntry(entry)` survives encode/decode |

---

### Step 2 — Daemon log capture (`ferro-wg-core/src/log_stream.rs` + `daemon.rs`)

**File:** `ferro-wg-core/src/log_stream.rs` (new)

#### `LogBroadcaster`

```rust
pub struct LogBroadcaster {
    sender: broadcast::Sender<LogEntry>,
    /// Ring buffer of recent entries replayed to new subscribers.
    history: Mutex<VecDeque<LogEntry>>,
    history_capacity: usize,
}

impl LogBroadcaster {
    /// Create a broadcaster with the given channel and history capacities.
    pub fn new(channel_capacity: usize, history_capacity: usize) -> Arc<Self> { ... }

    /// Publish a log entry. If the channel is full, older entries are
    /// dropped by the broadcast crate (receivers get `RecvError::Lagged`).
    /// History is always updated regardless of channel state.
    pub fn publish(&self, entry: LogEntry) { ... }

    /// Subscribe and snapshot current history atomically.
    /// Returns `(receiver, history_snapshot)`.
    pub fn subscribe(&self) -> (broadcast::Receiver<LogEntry>, Vec<LogEntry>) { ... }
}
```

#### `TuiTracingLayer`

```rust
/// A `tracing::Layer` that routes log events to a [`LogBroadcaster`].
pub struct TuiTracingLayer {
    broadcaster: Arc<LogBroadcaster>,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for TuiTracingLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: ...) {
        // Extract level, target, message; build LogEntry with
        // timestamp_ms = SystemTime::now() ms since UNIX_EPOCH.
        // connection_name is extracted from a structured field named
        // "connection_name" if present, otherwise None.
        self.broadcaster.publish(entry);
    }
}
```

#### `daemon.rs` — streaming handler

In `ferro-wg-core/src/daemon.rs`, the main accept loop gains a reference to
`Arc<LogBroadcaster>`. When `StreamLogs` is decoded:

```rust
DaemonCommand::StreamLogs { connection_name, min_level } => {
    let broadcaster = Arc::clone(&log_broadcaster);
    tokio::spawn(async move {
        stream_logs(writer, broadcaster, connection_name, min_level).await;
    });
    continue; // loop back to accept() immediately
}
```

`stream_logs()`:

```rust
async fn stream_logs(
    mut writer: OwnedWriteHalf,
    broadcaster: Arc<LogBroadcaster>,
    filter_connection: Option<String>,
    min_level: LogLevel,
) {
    let (mut rx, history) = broadcaster.subscribe();

    // Replay history (filtered).
    for entry in history.into_iter().filter(|e| passes_filter(e, &filter_connection, min_level)) {
        if send_entry(&mut writer, &entry).await.is_err() { return; }
    }

    // Forward live events.
    loop {
        match rx.recv().await {
            Ok(entry) => {
                if passes_filter(&entry, &filter_connection, min_level) {
                    if send_entry(&mut writer, &entry).await.is_err() { return; }
                }
            }
            Err(RecvError::Lagged(n)) => {
                // Synthesize a warning entry so the TUI user knows entries were dropped.
                let warn = lag_warning_entry(n);
                let _ = send_entry(&mut writer, &warn).await;
            }
            Err(RecvError::Closed) => return,
        }
    }
}
```

`daemon::run()` signature change:

```rust
pub async fn run(
    config: AppConfig,
    config_path: &Path,
    socket_path: &Path,
    log_broadcaster: Arc<LogBroadcaster>,
) -> Result<(), Box<dyn std::error::Error>>
```

#### Daemon binary wiring (`ferro-wg-daemon/src/main.rs`, `ferro-wg/src/cli.rs`)

```rust
let broadcaster = LogBroadcaster::new(512, 200);
let layer = TuiTracingLayer::new(Arc::clone(&broadcaster));
tracing_subscriber::registry()
    .with(tracing_subscriber::fmt::layer())  // existing stderr logging
    .with(layer)                              // new TUI broadcast layer
    .init();

daemon::run(config, &config_path, &socket_path, broadcaster).await?;
```

#### Tests

| Test | Assertion |
|------|-----------|
| `broadcaster_publish_and_receive` | Published entry arrives on subscriber's receiver |
| `broadcaster_history_replayed_on_subscribe` | 5 entries published before subscribe → subscriber receives all 5 as history |
| `broadcaster_history_capped` | 300 entries published with capacity 200 → subscriber history has exactly 200 |
| `broadcaster_lagged_receiver` | Publish 600 entries into a capacity-512 channel → receiver gets `RecvError::Lagged` |
| `stream_logs_filters_by_level` | `min_level: Warn` → DEBUG/INFO entries are suppressed before the writer |
| `stream_logs_filters_by_connection` | `filter_connection: Some("mia")` → entries with `connection_name: Some("ord01")` are skipped |
| `stream_logs_global_events_pass_connection_filter_none` | `filter_connection: None` → entries with `connection_name: None` are forwarded |

---

### Step 3 — TUI state: `LogBuffer` and new Actions (`ferro-wg-tui-core`)

**Files:** `src/state.rs`, `src/action.rs`

#### `LogBuffer`

```rust
/// Bounded ring buffer for daemon log entries.
///
/// Oldest entries are evicted when the buffer reaches capacity, ensuring
/// the TUI memory footprint is bounded regardless of daemon verbosity.
pub struct LogBuffer {
    entries: VecDeque<LogEntry>,
    capacity: usize,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self { ... }

    /// Append an entry, evicting the oldest if at capacity.
    pub fn push(&mut self, entry: LogEntry) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Iterate entries passing the given level and optional connection filter.
    /// `query` is a case-insensitive substring matched against the message.
    pub fn filtered<'a>(
        &'a self,
        min_level: LogLevel,
        connection: Option<&'a str>,
        query: &'a str,
    ) -> impl Iterator<Item = &'a LogEntry> { ... }

    pub fn len(&self) -> usize { ... }
    pub fn is_empty(&self) -> bool { ... }
}
```

#### `AppState` changes

Replace:
```rust
pub log_lines: Vec<String>,
```
With:
```rust
/// Bounded buffer of daemon log entries for the Logs tab.
pub log_buffer: LogBuffer,
/// Whether the log stream is currently connected to the daemon.
pub log_stream_connected: bool,
```

`AppState::new()` initialises `log_buffer: LogBuffer::new(1_000)` and
`log_stream_connected: false`.

#### New `Action` variants

```rust
/// Append a log entry received from the daemon stream.
AppendLog(LogEntry),
/// Set the minimum log level displayed in the Logs tab.
/// The buffer always stores all levels; this is a view filter only.
SetLogMinLevel(LogLevel),
/// Log stream connected to daemon (or reconnected after a gap).
LogStreamConnected,
/// Log stream disconnected from daemon.
LogStreamDisconnected,
```

#### `AppState::dispatch` additions

- `AppendLog(entry)` → `self.log_buffer.push(entry.clone())`
- `SetLogMinLevel(level)` → stored on `LogsComponent` (local component state, **not** `AppState`) — this Action is a signal to the component only; `dispatch` ignores it.
- `LogStreamConnected` → `self.log_stream_connected = true`
- `LogStreamDisconnected` → `self.log_stream_connected = false`

#### Tests

| Test | Assertion |
|------|-----------|
| `log_buffer_push_and_len` | Push 3 entries → `len() == 3` |
| `log_buffer_evicts_at_capacity` | Push `capacity + 1` entries → oldest evicted, `len() == capacity` |
| `log_buffer_filtered_by_level` | Mix of levels → `filtered(Info, None, "")` omits DEBUG/TRACE |
| `log_buffer_filtered_by_connection` | Mix of connections → `filtered(Trace, Some("mia"), "")` omits "ord01" entries |
| `log_buffer_filtered_by_query` | `filtered(Trace, None, "handshake")` matches only lines containing "handshake" |
| `log_buffer_filtered_global_events` | Entries with `connection_name: None` pass a `Some("mia")` connection filter (global events are always shown) |
| `dispatch_append_log` | `AppendLog(entry)` → `state.log_buffer.len() == 1` |
| `dispatch_log_stream_connected` | `LogStreamConnected` → `state.log_stream_connected == true` |
| `dispatch_log_stream_disconnected` | `LogStreamDisconnected` → `state.log_stream_connected == false` |
| `append_log_evicts_at_capacity` | Dispatch 1 001 `AppendLog` actions with capacity 1 000 → buffer len stays at 1 000 |

---

### Step 4 — TUI log stream task (`ferro-wg-tui/src/lib.rs`)

#### New `DaemonMessage` variants

```rust
enum DaemonMessage {
    StatusUpdate(Vec<PeerStatus>),
    CommandOk(String),
    CommandError(String),
    Unreachable,
    LogEntry(LogEntry),          // ← new
    LogStreamDisconnected,       // ← new
}
```

#### `spawn_log_stream()`

```rust
fn spawn_log_stream(
    tx: mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
) {
    tasks.spawn(async move {
        loop {
            match open_log_stream().await {
                Ok(mut reader) => {
                    let _ = tx.send(DaemonMessage::LogEntry(synthetic_connected_entry()));
                    // Read LogEntry messages until EOF.
                    let mut line = String::new();
                    loop {
                        line.clear();
                        match reader.read_line(&mut line).await {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {
                                if let Ok(DaemonResponse::LogEntry(entry)) =
                                    ipc::decode_message(&line)
                                {
                                    if tx.send(DaemonMessage::LogEntry(entry)).is_err() {
                                        return; // TUI shut down
                                    }
                                }
                            }
                        }
                    }
                }
                Err(_) => {} // daemon not running yet
            }
            let _ = tx.send(DaemonMessage::LogStreamDisconnected);
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}
```

`open_log_stream()` connects to the daemon socket and sends
`DaemonCommand::StreamLogs { connection_name: None, min_level: LogLevel::Debug }`,
returning a `BufReader` over the socket's read half.

The stream task is spawned **once** at TUI startup, alongside the status poll
infrastructure, and runs for the lifetime of the TUI session.

#### Event loop wiring

In the `daemon_rx.try_recv()` drain loop:

```rust
DaemonMessage::LogEntry(entry) => vec![Action::AppendLog(entry)],
DaemonMessage::LogStreamDisconnected => vec![Action::LogStreamDisconnected],
```

`spawn_log_stream(&daemon_tx, &mut tasks)` is called once after the channel is
created, before the event loop begins.

The synthetic `connected_entry()` dispatches `Action::LogStreamConnected` via a
`DaemonMessage::LogStreamConnected` variant (add this too) so the status bar
can show the stream state.

#### Tests

| Test | Assertion |
|------|-----------|
| `log_stream_disconnected_action_on_daemon_gone` | Mock server closes connection → `LogStreamDisconnected` action dispatched |
| `log_stream_reconnects_after_gap` | Mock server unavailable for 2 s then comes back → stream resumes |
| `log_stream_entries_dispatched_in_order` | Mock server sends 10 entries → `log_buffer` contains all 10 in order |
| `log_stream_does_not_affect_status_poll` | Log stream task running → status poll still fires every 250 ms independently |

---

### Step 5 — `LogsComponent` full implementation (`ferro-wg-tui-components/src/logs.rs`)

#### Local component state

```rust
pub struct LogsComponent {
    /// Lines scrolled above the bottom of the visible window (0 = pinned to bottom).
    scroll_offset: usize,
    /// Minimum log level to display in the rendered viewport.
    level_filter: LogLevel,
    /// Whether to show only the active connection's logs or all logs.
    connection_filter: ConnectionFilter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionFilter {
    All,
    ActiveConnection,
}
```

#### Key bindings

| Key | Behaviour |
|-----|-----------|
| `j` / `↓` | Scroll down (toward newer entries); clamp at 0 (auto-scroll) |
| `k` / `↑` | Scroll up (toward older entries); clamp at `visible_count - viewport_height` |
| `g` | Jump to oldest (max scroll_offset) |
| `G` | Jump to newest (`scroll_offset = 0`, re-enable auto-scroll) |
| `l` | Cycle `level_filter` via `LogLevel::next_filter_level()`; reset `scroll_offset` to 0 |
| `c` | Toggle `connection_filter` between `All` and `ActiveConnection`; reset `scroll_offset` to 0 |
| `/` | Emit `Action::EnterSearch` (reuse existing search infrastructure) |

Auto-scroll: when `scroll_offset == 0` and an `AppendLog` action arrives
(observed via `update()`), scroll stays at 0 so the new entry is immediately
visible. When `scroll_offset > 0` (user has scrolled up), the view freezes.

#### Render layout

```
┌─ Logs [INFO+] [mia] ───────────────────────────────────────────────────────┐
│ 12:34:01  INFO   Tunnel mia up, backend=boringtun                     │▲│  │
│ 12:34:02  DEBUG  Handshake complete with 198.51.100.1:51820           │ │  │
│ 12:34:05  WARN   Keepalive timeout; retrying                          │█│  │
│ 12:34:07  ERROR  Failed to decapsulate packet: invalid tag            │ │  │
│ ...                                                                   │▼│  │
└────────────────────────────────────────────────────────────────────────────┘
```

- Block title encodes active filters: `Logs [INFO+] [mia]` or `Logs [DEBUG+] [all]`
- Scrollbar uses `ratatui::widgets::Scrollbar` (vertical, right side)
- Level badge coloured spans using theme roles:
  - `ERROR` → `theme.error`
  - `WARN` → `theme.warning`
  - `INFO` → `theme.success`
  - `DEBUG` / `TRACE` → `theme.muted`
- Timestamp formatted from `entry.timestamp_ms` as `HH:MM:SS` (local time)
- Search highlight: query matches are bold/underline within the message span
- When `log_stream_connected == false`: a `[disconnected]` suffix in the title,
  message lines from the buffer still shown but a muted banner at the bottom:
  `"⚠ log stream disconnected — reconnecting…"`
- When buffer is empty: centred muted placeholder `"(no log entries yet)"`

#### `update()` hook

```rust
fn update(&mut self, action: &Action, _state: &AppState) {
    match action {
        Action::AppendLog(_) => {
            // Auto-scroll: if pinned to bottom, stay pinned.
            // (scroll_offset == 0 means pinned; nothing to change.)
        }
        Action::SetLogMinLevel(level) => {
            self.level_filter = *level;
            self.scroll_offset = 0;
        }
        _ => {}
    }
}
```

`handle_key()` emits `Action::SetLogMinLevel(self.level_filter.next_filter_level())`
when `l` is pressed — `AppState::dispatch` ignores it; `update()` on
`LogsComponent` applies it locally. This keeps level-filter state local to the
component (other components have no use for it).

#### Tests

| Test | Assertion |
|------|-----------|
| `logs_renders_empty_placeholder` | Empty buffer → placeholder text in buffer |
| `logs_renders_entries_newest_at_bottom` | 3 entries → oldest top, newest bottom |
| `logs_level_badge_colour` | ERROR entry → rendered span uses `theme.error` fg |
| `logs_scroll_up_moves_offset` | `↑` on 20 entries in 5-row viewport → `scroll_offset` increases |
| `logs_scroll_down_clamps_at_zero` | `↓` at `scroll_offset == 0` → offset stays 0 |
| `logs_g_key_jumps_to_top` | `g` → `scroll_offset == filtered_count - viewport_height` |
| `logs_G_key_pins_to_bottom` | `G` → `scroll_offset == 0` |
| `logs_append_does_not_unfreeze_scroll` | New entry arrives while `scroll_offset > 0` → offset unchanged |
| `logs_append_auto_scrolls_when_pinned` | New entry arrives while `scroll_offset == 0` → view updates (offset stays 0) |
| `logs_l_key_cycles_level` | `l` from INFO → `next_filter_level()` → level becomes WARN; entries below WARN hidden |
| `logs_c_key_toggles_connection_filter` | `c` → `ConnectionFilter::ActiveConnection`; `c` again → `All` |
| `logs_search_highlights_match` | Query "handshake" → matching span is bold in buffer |
| `logs_renders_disconnected_banner` | `state.log_stream_connected == false` → banner text present |
| `logs_title_reflects_filters` | `level_filter = Warn`, `connection_filter = ActiveConnection`, active connection "mia" → title contains `[WARN+] [mia]` |
| `logs_global_events_shown_in_connection_filter` | Entry with `connection_name: None` → visible even in `ActiveConnection` mode |

---

## Files Modified Summary

| File | Change |
|------|--------|
| `ferro-wg-core/src/ipc.rs` | `LogLevel`, `LogEntry`, `DaemonCommand::StreamLogs`, `DaemonResponse::LogEntry` |
| `ferro-wg-core/src/log_stream.rs` | **New** — `LogBroadcaster`, `TuiTracingLayer` |
| `ferro-wg-core/src/daemon.rs` | Accept `Arc<LogBroadcaster>`; `tokio::spawn` streaming handler for `StreamLogs` |
| `ferro-wg-core/src/lib.rs` | Re-export `log_stream` module |
| `ferro-wg-daemon/src/main.rs` | Create broadcaster, install `TuiTracingLayer`, pass to `daemon::run()` |
| `ferro-wg/src/cli.rs` | Same broadcaster wiring for `ferro-wg daemon` subcommand |
| `ferro-wg-tui-core/src/state.rs` | `LogBuffer`, replace `log_lines`, `log_stream_connected`, new dispatch arms |
| `ferro-wg-tui-core/src/action.rs` | `AppendLog`, `SetLogMinLevel`, `LogStreamConnected`, `LogStreamDisconnected` |
| `ferro-wg-tui/src/lib.rs` | `DaemonMessage::LogEntry` / `LogStreamDisconnected`, `spawn_log_stream()`, event loop wiring |
| `ferro-wg-tui-components/src/logs.rs` | Full `LogsComponent` rewrite with scroll, filters, search, scrollbar |

No changes to `ferro-wg-tui-components` other than `logs.rs`.
No changes to the Overview, Status, Peers, Config, or Compare components.

---

## Testing Strategy

### Unit tests

All in `#[cfg(test)]` blocks within the files enumerated above (42 tests total
across all steps).

### Integration tests (`ferro-wg-tui/tests/log_streaming.rs`)

These spin up a mock daemon that:
1. Binds a `UnixListener` on a temp path
2. Accepts a `StreamLogs` command
3. Streams `DaemonResponse::LogEntry` messages at controlled intervals

| Test | What it verifies |
|------|-----------------|
| `log_entries_appear_in_buffer_within_one_tick` | Mock sends entry → after 1 tick cycle, `state.log_buffer.len() == 1` |
| `log_stream_reconnects_after_daemon_restart` | Mock disconnects and reconnects → buffer continues to fill |
| `log_level_filter_applied_server_side` | Mock sends DEBUG entries; `min_level: Info` → TUI buffer receives zero DEBUG entries |
| `log_connection_filter_applied_server_side` | Mock sends "mia" and "ord01" entries; filter on "mia" → only "mia" entries in buffer |
| `history_replayed_on_connect` | Mock had 50 entries in history → TUI buffer has 50 entries immediately after connect |
| `buffer_capacity_respected_under_flood` | Mock streams 2 000 entries rapidly → `log_buffer.len() <= 1_000` at all times |

---

## Success Criteria

1. Daemon log entries appear in the Logs tab within 100 ms of emission.
2. Entries show correctly coloured level badges and `HH:MM:SS` timestamps.
3. `↑`/`↓`/`g`/`G` scroll correctly; auto-scroll re-engages on `G`.
4. `l` cycles level filter; lower-level entries disappear without buffer loss.
5. `c` toggles per-connection filtering; global daemon events are always shown.
6. `/` search highlights matches within visible lines.
7. Buffer wraps at 1 000 entries without stutter.
8. Stream reconnects within 2 s of daemon restart; `log_stream_connected` reflects state.
9. Status poll continues unaffected while the log stream runs.
10. `cargo test --workspace --all-features`, `cargo clippy --all-targets
    --all-features -- -D warnings -D clippy::pedantic`, and `cargo fmt --check`
    all pass.
