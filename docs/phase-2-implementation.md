# Phase 2: Multi-Connection Support — Implementation Plan

## Context

Phase 1 delivered live daemon integration: the TUI polls status every 250 ms,
dispatches up/down/backend-switch commands, and displays feedback. However, it
loads only the **first** connection from `AppConfig` and has no mechanism to
show, select, or manage the others.

The daemon (`TunnelManager`) already supports multiple simultaneous connections
via an internal `HashMap<String, ActiveConnection>`. Phase 2 surfaces that
capability in the TUI — making all configured connections visible, individually
selectable, and independently manageable.

**Done when:** every connection in `AppConfig` is visible in the TUI; each can
be brought up, torn down, and inspected independently; an aggregate Overview tab
shows at-a-glance health across all connections; and live stats update for all
connections simultaneously.

---

## Current Gaps

| Area | Today | After Phase 2 |
|------|-------|---------------|
| `ferro-wg/src/main.rs` | Passes first `WgConfig` to TUI | Passes full `AppConfig` |
| `AppState` | Holds one `WgConfig` + flat `Vec<PeerState>` | Holds `Vec<ConnectionView>` with per-connection state |
| TUI entry point | `run(WgConfig)` | `run(AppConfig)` |
| Status/Peers/Config tabs | Scoped to single connection | Scoped to selected connection |
| Overview | None | New tab: aggregate health table for all connections |
| Connection selector | None | Thin header band + `[`/`]` keybindings |
| `UpdatePeers` dispatch | Replaces entire peer list | Routes by connection name |
| `ConnectPeer` / `DisconnectPeer` | Works (name = connection name) | No IPC change needed |

---

## Architecture

```
AppConfig (all named connections)
    │
    ▼
AppState::new(AppConfig)
    connections: Vec<ConnectionView>   ← one per named connection
    selected_connection: usize         ← index of focused connection
    │
    ├─ Tab::Overview  → OverviewComponent    (all connections, summary table)
    ├─ Tab::Status    → StatusComponent      (active_connection().peers)
    ├─ Tab::Peers     → PeersComponent       (active_connection().config.peers)
    ├─ Tab::Compare   → CompareComponent     (active_connection())
    ├─ Tab::Config    → ConfigComponent      (active_connection().config)
    └─ Tab::Logs      → LogsComponent        (unchanged)
    │
    └─ ConnectionBarComponent  (between tab bar and content, >1 connection only)
           shows:  ◀  [1] mia ●  [2] tus1 ○  [3] ord01 ●  ▶
```

### Data Model

```
ConnectionView {
    name:              String                 ← key from AppConfig
    config:            WgConfig               ← static config (peers, interface)
    status:            Option<ConnectionStatus>  ← None until first poll
    selected_peer_row: usize                  ← per-connection table cursor
}

ConnectionStatus {
    state:     ConnectionState   ← enum, not bool
    backend:   BackendKind
    stats:     TunnelStats
    endpoint:  Option<String>
    interface: Option<String>
}

ConnectionState {   ← enum replacing bare `bool`
    Connected,
    Disconnected,
}
```

`ConnectionStatus` maps 1:1 to the fields of `PeerStatus` that come back from
the daemon. `PeerStatus.name` is already the connection name — no IPC change is
required. `PeerStatus.connected: bool` maps to `ConnectionState::Connected` /
`ConnectionState::Disconnected`. A `Connecting` variant is **not** added in
Phase 2 because the daemon has no such state; if it becomes necessary (e.g.
for handshake-in-progress feedback), it will be added in Phase 4 alongside the
lifecycle management work.

### Key Invariants

- `selected_connection` is always a valid index into `connections`, or 0 when
  `connections` is empty. `connections.get(selected_connection)` returns `None`
  for empty configs, which all callers handle via `active_connection()`.
- `selected_connection` is clamped to `connections.len().saturating_sub(1)`
  whenever `connections` changes (currently only at startup — see Static Config
  Assumption below).
- `selected_peer_row` for each `ConnectionView` is clamped to
  `config.peers.len().saturating_sub(1)` on every `NextRow` / `PrevRow`
  dispatch, guarding against stale cursors if peers change.
- When `connections` is empty, all content tabs render a "no connections
  configured" placeholder. The Overview tab is always renderable (it shows an
  empty table).

### Vec vs HashMap for Connection Storage

`connections` is stored as `Vec<ConnectionView>` (not `HashMap<String, ConnectionView>`)
because the primary access pattern is **index-based**: `selected_connection: usize` into an
ordered list. A `HashMap` would complicate stable ordering and `SelectNextConnection` wrapping.

`UpdatePeers` iterates incoming `PeerStatus` entries and finds each matching `ConnectionView`
by name — O(n) per entry, O(n²) overall. With the bounded connection count (<10 for Phase 2),
this is at most ~100 comparisons per poll cycle and is not a practical concern. If Phase 4
relaxes the static-config assumption and allows hundreds of dynamic connections, this should
be revisited (e.g. maintain a `HashMap<&str, usize>` name→index alongside the Vec).

### Static Config Assumption

`AppConfig` is treated as **read-only** for the lifetime of the TUI session.
No hot-reload or dynamic add/remove of connections is performed in Phase 2.
`UpdatePeers` matches incoming `PeerStatus` entries to `ConnectionView` by name;
unrecognised names (connections added to the daemon config after TUI startup)
are silently ignored. This assumption is explicitly documented here so that
Phase 4 (lifecycle management) can revisit it.

### No IPC Changes Required

`TunnelManager::status()` already returns one `PeerStatus` per named connection,
with `PeerStatus.name` equal to the connection key in `AppConfig`. The daemon
already supports `Up { peer_name: Some(name) }` and `Down { peer_name: Some(name) }`
for per-connection control. Phase 2 is entirely a TUI-layer change.

---

## Dependency Graph

```
ferro-wg-core          ← unchanged
    ↑
ferro-wg-tui-core      ← Step 1: ConnectionView, AppState redesign, new Actions
    ↑
ferro-wg-tui-components ← Step 3: ConnectionBarComponent (new)
                           Step 4: OverviewComponent (new)
                           Step 5: scoped Status/Peers/Config/Compare
    ↑
ferro-wg-tui           ← Step 2: run(AppConfig), layout wiring
    ↑
ferro-wg               ← Step 2: pass AppConfig instead of first WgConfig
```

---

## Implementation Steps

### Step 1 — Core data model (`ferro-wg-tui-core`)

**Files:** `src/state.rs`, `src/action.rs`

#### 1a — Add `ConnectionState`, `ConnectionView`, and `ConnectionStatus`

```rust
/// Whether a connection tunnel is currently active.
///
/// A plain `bool` is insufficient here because it cannot express the
/// absence of data (`None` on `ConnectionStatus`). The enum is kept
/// minimal for Phase 2; a `Connecting` variant may be added in Phase 4
/// if the daemon gains handshake-in-progress signalling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Disconnected,
}

/// Static config and live status for one named connection.
#[derive(Debug, Clone)]
pub struct ConnectionView {
    /// Connection name as it appears in `AppConfig` (e.g. `"mia"`).
    pub name: String,
    /// Static WireGuard config (interface + peers).
    pub config: WgConfig,
    /// Live status from the last daemon poll; `None` until the first poll
    /// completes.
    pub status: Option<ConnectionStatus>,
    /// Which peer row is selected in the Status/Peers tabs for this
    /// connection. Preserved when switching away and back.
    pub selected_peer_row: usize,
}

/// Live status for one connection, sourced from a `PeerStatus` daemon response.
#[derive(Debug, Clone)]
pub struct ConnectionStatus {
    pub state: ConnectionState,
    pub backend: BackendKind,
    pub stats: TunnelStats,
    pub endpoint: Option<String>,
    pub interface: Option<String>,
}
```

#### 1b — Redesign `AppState`

Replace `wg_config: WgConfig` and `peers: Vec<PeerState>` with:

```rust
pub struct AppState {
    pub running: bool,
    pub active_tab: Tab,
    pub input_mode: InputMode,
    pub search_query: String,
    /// All configured connections in display order (sorted by name).
    pub connections: Vec<ConnectionView>,
    /// Index into `connections` for the currently focused connection.
    /// Always 0 when `connections` is empty.
    pub selected_connection: usize,
    pub log_lines: Vec<String>,
    pub theme: Theme,
    pub daemon_connected: bool,
    pub feedback: Option<Feedback>,
}
```

`AppState::new(app_config: AppConfig)` builds a `ConnectionView` (with
`status: None`) for every entry in `app_config.connections`, sorted
alphabetically by name. An empty `AppConfig` produces `connections: vec![]`
and `selected_connection: 0`; all accessors return `None` gracefully.

Add accessor helpers:

```rust
impl AppState {
    /// Returns the currently focused connection, if any.
    pub fn active_connection(&self) -> Option<&ConnectionView> {
        self.connections.get(self.selected_connection)
    }

    /// Returns the currently focused connection mutably.
    pub fn active_connection_mut(&mut self) -> Option<&mut ConnectionView> {
        self.connections.get_mut(self.selected_connection)
    }
}
```

#### 1c — New `Action` variants

```rust
pub enum Action {
    // ... existing variants unchanged ...

    /// Focus the next connection in the list (wraps).
    SelectNextConnection,
    /// Focus the previous connection in the list (wraps).
    SelectPrevConnection,
    /// Focus a specific connection by index.
    SelectConnection(usize),
}
```

#### 1d — Update `AppState::dispatch`

- `SelectNextConnection` — if `connections.is_empty()`, no-op; otherwise
  `selected_connection = (selected_connection + 1) % connections.len()`; clear
  `search_query`. The `is_empty()` guard is required before the modulo to avoid
  a division-by-zero panic.
- `SelectPrevConnection` — if `connections.is_empty()`, no-op; otherwise
  `selected_connection = selected_connection.checked_sub(1).unwrap_or(connections.len() - 1)`;
  clear `search_query`.
- `SelectConnection(i)` — silently ignore if `i >= connections.len()`; otherwise `selected_connection = i`.
- `UpdatePeers(Vec<PeerStatus>)` — for each `PeerStatus`, find the matching
  `ConnectionView` by name using `connections.iter_mut().find(|c| c.name == s.name)`.
  Unrecognised names are skipped. Set `daemon_connected = true`. Clamp
  `selected_connection` to `connections.len().saturating_sub(1)` after the loop
  (defensive; connections are static in Phase 2 but the invariant must hold).
- `NextRow` — on `active_connection_mut()`, increment `selected_peer_row`,
  clamp to `conn.config.peers.len().saturating_sub(1)`.
- `PrevRow` — on `active_connection_mut()`, decrement with
  `checked_sub(1).unwrap_or(0)`.

#### 1e — Add `Tab::Overview`

```rust
pub enum Tab {
    Overview,  // ← new, index 0
    Status,
    Peers,
    Compare,
    Config,
    Logs,
}
```

Update `Tab::index()`, `Tab::from_index()`, and any `match` exhaustion sites.
`Tab::Overview` is the default tab in `AppState::new`.

The existing number-key shortcuts (`'1'`–`'5'` → `SelectTab`) shift by one:
`'1'` → Overview, `'2'` → Status, …, `'6'` → Logs.

#### 1f — Tests

| Test | Assertion |
|------|-----------|
| `connections_sorted_on_new` | 3 unsorted connections → sorted by name |
| `new_empty_config` | `connections` empty, `selected_connection = 0`, `active_connection()` returns `None` |
| `update_peers_routes_by_name` | Two `PeerStatus` entries → each updates the correct `ConnectionView` |
| `update_peers_partial` | Update for one connection leaves the other's status unchanged |
| `update_peers_unknown_name_ignored` | `PeerStatus` with unrecognised name → no panic, no state change |
| `select_next_wraps` | `SelectNextConnection` at last index → wraps to 0 |
| `select_prev_wraps` | `SelectPrevConnection` at index 0 → wraps to last |
| `select_next_empty` | `SelectNextConnection` with empty connections → no panic |
| `select_connection_out_of_bounds` | `SelectConnection(99)` on 2-connection list → silently ignored |
| `next_prev_row_per_connection` | `NextRow` on connection 0 does not change connection 1's `selected_peer_row` |
| `next_row_clamps_at_end` | `NextRow` past last peer → stays at last index |
| `prev_row_clamps_at_zero` | `PrevRow` at 0 → stays at 0 |

---

### Step 2 — TUI entry point (`ferro-wg-tui`, `ferro-wg`)

**Files:** `ferro-wg-tui/src/lib.rs`, `ferro-wg/src/main.rs`

#### 2a — Change `run()` signature

```rust
// Before
pub async fn run(wg_config: WgConfig) -> Result<(), Box<dyn std::error::Error>>

// After
pub async fn run(app_config: AppConfig) -> Result<(), Box<dyn std::error::Error>>
```

Replace `AppState::new(wg_config)` with `AppState::new(app_config)` inside
`event_loop`.

#### 2b — Update `ferro-wg/src/main.rs`

```rust
// Remove:
let first = app_config.connections.values().next()
    .ok_or("no connections configured")?
    .clone();
rt.block_on(ferro_wg_tui::run(first))?;

// Replace with:
rt.block_on(ferro_wg_tui::run(app_config))?;
```

The `"no connections configured"` guard is removed — an empty `AppConfig` is
valid and the TUI renders a placeholder. Users are expected to import
connections before launching the TUI, but it is not a hard error.

#### 2c — Global keybindings in `handle_global_key`

```rust
KeyCode::Char('[') => Some(Action::SelectPrevConnection),
KeyCode::Char(']') => Some(Action::SelectNextConnection),
```

Direct connection selection by number is **not** implemented via `Ctrl+N` in
Phase 2. The existing `'1'`–`'6'` tab shortcuts (shifted by one to accommodate
the new Overview tab) occupy unmodified number keys; `Ctrl+N` would conflict
with terminal emulator shortcuts on some platforms. Direct connection selection
by index is deferred to Phase 7 (UX Polish) where keybindings can be designed
holistically.

---

### Step 3 — `ConnectionBarComponent` (`ferro-wg-tui-components`)

**File:** `src/connection_bar.rs` (new)

A thin horizontal strip (1 row) rendered between the tab bar and content when
`state.connections.len() > 1`. Hidden when there is only one connection (no
visual clutter for single-connection users).

Renders:

```
 Connections:  ◀  [1] mia ●  [2] tus1 ○  [3] ord01 ●  ▶
```

- Selected connection name is bold / highlighted with the accent colour.
- `●` green = `ConnectionState::Connected`
- `○` dim = `ConnectionState::Disconnected`
- `?` yellow = `status: None` (not yet polled)
- `◀` / `▶` scroll indicators appear when names overflow terminal width.

#### Layout

The vertical layout in `ferro-wg-tui/src/lib.rs` always allocates the connection
bar slot, using `Constraint::Length(0)` when hidden. This avoids a full layout
recalculation (and the associated flicker) when the connection count crosses the
1/2 boundary mid-session.

```
Constraint::Length(3),                                         // tab bar
Constraint::Length(if connections.len() > 1 { 1 } else { 0 }), // connection bar
Constraint::Min(0),                                            // content
Constraint::Length(3),                                         // status bar
```

#### Tests

| Test | Assertion |
|------|-----------|
| `connection_bar_hidden_single` | 1 connection → bar height is 0 |
| `connection_bar_renders_all_names` | 3 connections → all three names in buffer |
| `connection_bar_highlights_selected` | `selected_connection = 1` → second name is bold |
| `connection_bar_not_polled_shows_question_mark` | `status: None` → `?` indicator |

---

### Step 4 — `OverviewComponent` (`ferro-wg-tui-components`)

**File:** `src/overview.rs` (new)

A table with one row per configured connection. The cursor (highlight) follows
`state.selected_connection`.

| Column | Source | When `status: None` |
|--------|--------|---------------------|
| # | 1-based index | — |
| Name | `ConnectionView.name` | name |
| Status | `ConnectionStatus.state` | `—` (not polled) |
| Backend | `ConnectionStatus.backend` | `—` |
| Interface | `ConnectionStatus.interface` | `—` |
| Tx | `stats.tx_bytes` | `—` |
| Rx | `stats.rx_bytes` | `—` |
| Last Handshake | `stats.last_handshake` | `—` |

Status column values: `● Connected` (green) / `○ Disconnected` (dim) / `—` (not yet polled, grey).
There is no "Connecting" state in Phase 2 — if the daemon does not report
a connection as connected, it is disconnected.

**Interaction:**

- `↑` / `↓` (or `k` / `j`) → `SelectConnection(i)`
- `Enter` or `→` → `SelectTab(Tab::Status)` for the highlighted connection
- Search filters connection names (same infrastructure as other tabs)

#### Tests

| Test | Assertion |
|------|-----------|
| `overview_renders_all_connections` | 3 connections → all names in buffer |
| `overview_empty_config` | 0 connections → no panic, empty table |
| `overview_selected_row_highlighted` | `selected_connection = 1` → second row highlighted |
| `overview_shows_not_polled_placeholder` | `status: None` → `—` in Status/Tx/Rx columns |
| `overview_connected_shows_green_indicator` | `Connected` → `●` |
| `overview_disconnected_shows_grey_indicator` | `Disconnected` → `○` |
| `overview_key_down_dispatches_select` | `↓` → `SelectConnection(next)` |
| `overview_key_down_wraps` | `↓` at last row → `SelectConnection(0)` |
| `overview_enter_switches_tab` | `Enter` → `SelectTab(Tab::Status)` |

---

### Step 5 — Scope existing tabs to `active_connection`

**Files:** `src/status.rs`, `src/peers.rs`, `src/config.rs`, `src/compare.rs`

All four tabs currently read from flat `AppState` fields that no longer exist
after Step 1. Each is updated to use `state.active_connection()` and to render
a placeholder on `None`.

#### Shared placeholder helper

```rust
pub fn render_no_connection_placeholder(frame: &mut Frame, area: Rect) {
    // centred grey text: "No connections configured."
}
```

#### Status tab

```rust
let Some(conn) = state.active_connection() else {
    render_no_connection_placeholder(frame, area);
    return;
};
let selected = conn.selected_peer_row;
```

#### Peers / Config / Compare tabs

Same `let Some(conn) = ...` guard. Peers and Config read from `conn.config`.
Compare passes `conn` to the backend availability check.

#### `selected_peer_row` safety

`NextRow` and `PrevRow` clamp `selected_peer_row` within `config.peers.len()`
(Step 1d). Components never need to do additional bounds checking; they can
index `conn.config.peers[conn.selected_peer_row]` directly after confirming
`!conn.config.peers.is_empty()`.

#### Tests

| Test | Assertion |
|------|-----------|
| `status_renders_active_connection_peers` | Status table rows match active connection's peers |
| `status_no_connection_shows_placeholder` | `connections` empty → placeholder rendered |
| `peers_renders_active_connection_config` | Peers table rows match active connection's config |
| `config_renders_active_connection_interface` | Interface fields match active connection |

---

### Step 6 — Wire layout and tab routing (`ferro-wg-tui`)

**File:** `src/lib.rs`

#### Layout

```rust
let chunks = Layout::vertical([
    Constraint::Length(3),
    Constraint::Length(if state.connections.len() > 1 { 1 } else { 0 }),
    Constraint::Min(0),
    Constraint::Length(3),
]).split(frame.area());

tab_bar.render(frame, chunks[0], false, &state);
if state.connections.len() > 1 {
    connection_bar.render(frame, chunks[1], false, &state);
}
components[state.active_tab.index()].render(frame, chunks[2], true, &state);
status_bar.render(frame, chunks[3], false, &state);
```

#### Component vec

```rust
let mut components: Vec<Box<dyn Component>> = vec![
    Box::new(OverviewComponent::new()),   // Tab::Overview (index 0)
    Box::new(StatusComponent::new()),
    Box::new(PeersComponent::new()),
    Box::new(CompareComponent::new()),
    Box::new(ConfigComponent::new()),
    Box::new(LogsComponent::new()),
];
```

#### Daemon command context

`maybe_spawn_command` already sends the connection name from the `Action`
string. `ConnectPeer(name)` / `DisconnectPeer(name)` pass the connection key —
no change required.

---

## Files Modified Summary

| File | Change |
|------|--------|
| `ferro-wg-tui-core/src/state.rs` | `ConnectionState`, `ConnectionView`, `ConnectionStatus`, `AppState` redesign, accessors, `AppState::new(AppConfig)`, updated dispatch with bounds clamping |
| `ferro-wg-tui-core/src/action.rs` | `SelectNextConnection`, `SelectPrevConnection`, `SelectConnection(usize)` |
| `ferro-wg-tui-components/src/connection_bar.rs` | **New** — `ConnectionBarComponent` |
| `ferro-wg-tui-components/src/overview.rs` | **New** — `OverviewComponent` |
| `ferro-wg-tui-components/src/status.rs` | Use `active_connection()`, `ConnectionState` |
| `ferro-wg-tui-components/src/peers.rs` | Use `active_connection()` |
| `ferro-wg-tui-components/src/config.rs` | Use `active_connection()` |
| `ferro-wg-tui-components/src/compare.rs` | Use `active_connection()` |
| `ferro-wg-tui-components/src/lib.rs` | Export new components and `render_no_connection_placeholder` |
| `ferro-wg-tui/src/lib.rs` | `run(AppConfig)`, updated layout, component vec with Overview, `[`/`]` keybindings |
| `ferro-wg/src/main.rs` | Pass `AppConfig` to `run()` directly |

No changes to `ferro-wg-core` (IPC, TunnelManager, config).

---

## Testing Strategy

### Unit tests

All in `ferro-wg-tui-core/src/state.rs` and component `#[cfg(test)]` blocks.
The full list is enumerated under each step above (31 tests total).

### Integration tests (required for Phase 2 — not deferred)

These live in `ferro-wg-tui/tests/multi_connection.rs` and use a mock daemon
helper that serialises `DaemonResponse::Status(Vec<PeerStatus>)` over a
`UnixListener` bound to a temp path.

| Test | What it verifies |
|------|-----------------|
| `two_connections_visible_after_tick` | After one 250 ms tick, `AppState.connections` reflects both mock connections |
| `up_one_connection_only_updates_that_status` | `ConnectPeer("mia")` → only `mia.status` becomes `Connected` |
| `down_all_clears_all_statuses` | `DaemonCommand::Down { peer_name: None }` → all `ConnectionState::Disconnected` |
| `zero_connections_no_panic` | Mock returns empty `Vec<PeerStatus>` → TUI renders without panic |
| `rapid_connection_switching` | 100 `SelectNextConnection` dispatches in sequence → `selected_connection` in-bounds |

---

## Success Criteria

1. All connections in `AppConfig` appear in the Overview tab with correct names.
2. `[` and `]` cycle between connections.
3. Status, Peers, and Config tabs show data scoped to the selected connection.
4. Switching connections restores the previous cursor position for each.
5. Connection bar is hidden (zero height, no flicker) when only one connection
   is configured — no regression for single-connection users.
6. `u` / `d` / `b` keys act on the currently selected connection.
7. Live stats update for all connections simultaneously.
8. `selected_peer_row` never goes out of bounds regardless of peer list changes.
9. `selected_connection` is always a valid index (or 0 for empty config).
10. `cargo test --workspace --all-features`, `cargo clippy --all-targets
    --all-features -- -D warnings -D clippy::pedantic`, and `cargo fmt --check`
    all pass.
