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
    name:              String           ← key from AppConfig
    config:            WgConfig         ← static config (peers, interface)
    status:            Option<ConnectionStatus>  ← None until first poll
    selected_peer_row: usize            ← per-connection table cursor
}

ConnectionStatus {
    connected:   bool
    backend:     BackendKind
    stats:       TunnelStats
    endpoint:    Option<String>
    interface:   Option<String>
}
```

`ConnectionStatus` maps 1:1 to the fields of `PeerStatus` that come back from
the daemon. `PeerStatus.name` is already the connection name — no IPC change is
required.

### Key Invariants

- `selected_connection` is always a valid index into `connections` (clamped on
  `UpdatePeers` if connections are added or removed).
- When `connections` is empty, all content tabs render a "no connections
  configured" placeholder. The Overview tab is always renderable.
- `selected_peer_row` is per-connection, so switching connections restores the
  previous cursor position for each one.

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

#### 1a — Add `ConnectionView` and `ConnectionStatus`

```rust
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

/// Live status for one connection, sourced from `PeerStatus` daemon response.
#[derive(Debug, Clone)]
pub struct ConnectionStatus {
    pub connected: bool,
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
    pub selected_connection: usize,
    pub log_lines: Vec<String>,
    pub theme: Theme,
    pub daemon_connected: bool,
    pub feedback: Option<Feedback>,
}
```

Add constructor `AppState::new(app_config: AppConfig)` that builds a
`ConnectionView` (with `status: None`) for every entry in
`app_config.connections`, sorted alphabetically by name.

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

- `Action::SelectNextConnection` — increment `selected_connection` modulo
  `connections.len()`, reset `search_query`, emit no side-effects.
- `Action::SelectPrevConnection` — decrement (wrapping), same cleanup.
- `Action::SelectConnection(i)` — bounds-check `i`, set
  `selected_connection = i`.
- `Action::UpdatePeers(Vec<PeerStatus>)` — **route by name**: for each
  `PeerStatus`, find the matching `ConnectionView` by `status.name == view.name`
  and update its `ConnectionStatus`. Set `daemon_connected = true`. Update
  `selected_connection` to remain in-bounds after any structural change.
- `Action::NextRow` / `PrevRow` — operate on
  `active_connection_mut().selected_peer_row` instead of a top-level field.

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

#### 1f — Tests

- `connections_sorted_on_new`: `AppState::new` with 3 connections, verify order.
- `update_peers_routes_by_name`: two `PeerStatus` entries with different names
  update the correct `ConnectionView`.
- `update_peers_partial`: status update for only one of two connections leaves
  the other unchanged.
- `select_next_wraps`: `SelectNextConnection` past end wraps to index 0.
- `select_prev_wraps`: `SelectPrevConnection` at index 0 wraps to last.
- `select_connection_out_of_bounds`: `SelectConnection(99)` on a 2-connection
  list is silently ignored.
- `next_prev_row_per_connection`: advancing `NextRow` on connection 0 does not
  affect connection 1's `selected_peer_row`.

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

Remove the first-connection extraction:

```rust
// Remove:
let first = app_config.connections.values().next()
    .ok_or("no connections configured")?
    .clone();
rt.block_on(ferro_wg_tui::run(first))?;

// Replace with:
rt.block_on(ferro_wg_tui::run(app_config))?;
```

#### 2c — Global keybindings in `event_loop`

Add to `handle_global_key`:

```rust
KeyCode::Char('[') => Some(Action::SelectPrevConnection),
KeyCode::Char(']') => Some(Action::SelectNextConnection),
KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::CONTROL) => {
    Some(Action::SelectConnection(0))
}
// ... Ctrl+2 through Ctrl+9 for up to 9 connections
```

The existing number keys (`'1'`–`'5'`) already select tabs; `Ctrl+N` selects
connection N without conflicting.

---

### Step 3 — `ConnectionBarComponent` (`ferro-wg-tui-components`)

**File:** `src/connection_bar.rs` (new)

A thin horizontal strip (1 row) rendered between the tab bar and content when
`state.connections.len() > 1`. Hidden when there is only one connection (no
visual clutter for the common single-connection case).

Renders:

```
 Connections:  ◀  [1] mia ●  [2] tus1 ○  [3] ord01 ●  ▶   [/] search  [[]/]] switch
```

- Selected connection name is bold / highlighted with the accent colour.
- `●` (green) = connected, `○` (dim) = disconnected, `?` (yellow) = not yet
  polled.
- `◀` / `▶` appear when there are more connections than fit in the terminal
  width (scrollable list, not paginated).

#### Layout impact

The vertical layout in `ferro-wg-tui/src/lib.rs` gains an optional row:

```
┌─────────────────────┐  height 3   Tab bar
├─────────────────────┤  height 1   Connection bar (hidden if ≤1 connection)
├─────────────────────┤  height *   Content
└─────────────────────┘  height 3   Status bar
```

Use `Constraint::Length(if show_connection_bar { 1 } else { 0 })` so the bar
disappears cleanly.

#### Tests

- `connection_bar_hidden_single`: `render()` with 1 connection emits a zero-
  height area (or is not called).
- `connection_bar_renders_all_names`: with 3 connections, all three names appear
  in rendered output.
- `connection_bar_highlights_selected`: selected connection name is rendered
  with bold modifier.

---

### Step 4 — `OverviewComponent` (`ferro-wg-tui-components`)

**File:** `src/overview.rs` (new)

A table with one row per configured connection, always showing all connections
regardless of `selected_connection`. The cursor (highlight) follows
`state.selected_connection`.

| Column | Source | Notes |
|--------|--------|-------|
| # | index | 1-based |
| Name | `ConnectionView.name` | |
| Status | `ConnectionStatus.connected` | `● Connected` / `○ Disconnected` / `⟳ Connecting` |
| Backend | `ConnectionStatus.backend` | `boringtun` / `neptun` / `gotatun` |
| Interface | `ConnectionStatus.interface` | `utun4`, `wg0`, etc. |
| Tx | `stats.tx_bytes` | Human-readable (KB/MB/GB) |
| Rx | `stats.rx_bytes` | Human-readable |
| Last Handshake | `stats.last_handshake` | Relative: `3s ago`, `2m ago`, `—` |

**Interaction:**

- `↑` / `↓` (or `k` / `j`) — move cursor, dispatches `SelectConnection(i)`.
- `Enter` or `→` — jump to the Status tab for the selected connection.
- Search from the status bar filters connection names (same search
  infrastructure as other tabs).

#### Tests

- `overview_renders_all_connections`: 3 connections all appear in rendered
  buffer.
- `overview_selected_row_highlighted`: `selected_connection = 1` highlights
  second row.
- `overview_shows_not_polled_placeholder`: `ConnectionStatus = None` renders
  `—` in status/stats columns.
- `overview_key_down_dispatches_select`: pressing `↓` emits
  `SelectConnection(next)`.
- `overview_enter_switches_tab`: pressing `Enter` emits `SelectTab(Tab::Status)`.

---

### Step 5 — Scope existing tabs to `active_connection`

**Files:** `src/status.rs`, `src/peers.rs`, `src/config.rs`, `src/compare.rs`

All four tabs currently read from flat `AppState` fields that no longer exist
after Step 1. Update each to use `state.active_connection()`.

#### Status tab

```rust
// Before
let peers = &state.peers;
let selected = state.selected_row;

// After
let Some(conn) = state.active_connection() else {
    render_no_connection_placeholder(frame, area);
    return;
};
let peers_from_config = &conn.config.peers;
let status = conn.status.as_ref();
let selected = conn.selected_peer_row;
```

The table combines static peer config (name, endpoint, allowed IPs) with live
status (connected, stats) by peer index, matching the existing rendering logic.

#### Peers tab

Same pattern — use `conn.config.peers` for the table data.

#### Config tab

```rust
let Some(conn) = state.active_connection() else { /* placeholder */ return; };
// render conn.config.interface fields
```

Show the connection name as a sub-heading above the interface block.

#### Compare tab

Pass `state.active_connection()` for the interface being compared. Phase 5
will expand this — for now, scope it correctly so the refactor is complete.

#### "No connection" placeholder

Add a shared helper in the crate:

```rust
pub fn render_no_connection_placeholder(frame: &mut Frame, area: Rect) {
    // centred grey text: "No connections configured."
}
```

All four tabs call it when `active_connection()` returns `None`.

#### Tests (each tab)

- `status_renders_active_connection_peers`
- `status_no_connection_shows_placeholder`
- `peers_renders_active_connection_config`
- `config_renders_active_connection_interface`

---

### Step 6 — Wire layout and tab routing (`ferro-wg-tui`)

**File:** `src/lib.rs`

#### Layout

```rust
let show_connection_bar = state.connections.len() > 1;
let bar_height = if show_connection_bar { 1 } else { 0 };

let chunks = Layout::vertical([
    Constraint::Length(3),           // tab bar
    Constraint::Length(bar_height),  // connection bar
    Constraint::Min(0),              // content
    Constraint::Length(3),           // status bar
]).split(frame.area());

tab_bar.render(frame, chunks[0], false, &state);
if show_connection_bar {
    connection_bar.render(frame, chunks[1], false, &state);
}
components[state.active_tab.index()].render(frame, chunks[2], true, &state);
status_bar.render(frame, chunks[3], false, &state);
```

#### Component vec

The component vec gains `OverviewComponent` at index 0:

```rust
let mut components: Vec<Box<dyn Component>> = vec![
    Box::new(OverviewComponent::new()),   // Tab::Overview
    Box::new(StatusComponent::new()),
    Box::new(PeersComponent::new()),
    Box::new(CompareComponent::new()),
    Box::new(ConfigComponent::new()),
    Box::new(LogsComponent::new()),
];
```

#### Daemon command context

`maybe_spawn_command` sends `DaemonCommand::Up { peer_name: Some(name) }` where
`name` comes from the `Action`. For `ConnectPeer(name)` / `DisconnectPeer(name)`
the name is the connection key — this is already correct. No change required.

---

## Files Modified Summary

| File | Change |
|------|--------|
| `ferro-wg-tui-core/src/state.rs` | `ConnectionView`, `ConnectionStatus`, `AppState` redesign, accessors, `AppState::new(AppConfig)`, updated dispatch |
| `ferro-wg-tui-core/src/action.rs` | `SelectNextConnection`, `SelectPrevConnection`, `SelectConnection(usize)` |
| `ferro-wg-tui-core/src/lib.rs` (if exists) | Re-export new types |
| `ferro-wg-tui-components/src/connection_bar.rs` | **New** — `ConnectionBarComponent` |
| `ferro-wg-tui-components/src/overview.rs` | **New** — `OverviewComponent` |
| `ferro-wg-tui-components/src/status.rs` | Use `active_connection()` |
| `ferro-wg-tui-components/src/peers.rs` | Use `active_connection()` |
| `ferro-wg-tui-components/src/config.rs` | Use `active_connection()` |
| `ferro-wg-tui-components/src/compare.rs` | Use `active_connection()` |
| `ferro-wg-tui-components/src/lib.rs` | Export new components |
| `ferro-wg-tui/src/lib.rs` | `run(AppConfig)`, layout with connection bar, component vec with Overview, `[`/`]` keybindings |
| `ferro-wg/src/main.rs` | Pass `AppConfig` to `run()` directly |

No changes to `ferro-wg-core` (IPC, TunnelManager, config) — the daemon layer
already supports everything Phase 2 requires.

---

## Testing Strategy

### Unit tests (in-crate, no daemon)

All in `ferro-wg-tui-core/src/state.rs` and component `#[cfg(test)]` blocks:

| Test | What it verifies |
|------|-----------------|
| `connections_sorted_on_new` | `AppState::new` sorts by name |
| `update_peers_routes_by_name` | `UpdatePeers` updates the correct `ConnectionView` |
| `update_peers_partial` | Other connections unchanged on partial update |
| `select_next_wraps` | `SelectNextConnection` wraps at end |
| `select_prev_wraps` | `SelectPrevConnection` wraps at start |
| `select_connection_bounds` | Out-of-range index silently ignored |
| `next_prev_row_per_connection` | `NextRow` / `PrevRow` isolated per connection |
| `overview_renders_all_connections` | All names present in render buffer |
| `overview_selected_row_highlighted` | Correct row highlighted |
| `overview_shows_not_polled_placeholder` | `None` status renders `—` |
| `overview_key_down_dispatches_select` | `↓` emits `SelectConnection` |
| `overview_enter_switches_tab` | `Enter` emits `SelectTab(Tab::Status)` |
| `connection_bar_hidden_single` | Bar not rendered with 1 connection |
| `connection_bar_renders_all_names` | All names present with 3 connections |
| `connection_bar_highlights_selected` | Selected name is bold |
| `status_renders_active_connection_peers` | Status table uses active connection |
| `status_no_connection_shows_placeholder` | Placeholder when empty |
| `peers_renders_active_connection_config` | Peers table uses active connection |
| `config_renders_active_connection_interface` | Config uses active connection |

### Integration tests (future Phase 2 PR, requires daemon)

- Spawn daemon with two mock connections; verify TUI `AppState` reflects both
  after first tick.
- Bring up one connection; verify only that connection's `ConnectionStatus`
  shows `connected = true`.

---

## Success Criteria

1. All connections in `AppConfig` appear in the Overview tab with correct names.
2. `[` and `]` cycle between connections; `Ctrl+N` selects connection N.
3. Status, Peers, and Config tabs show data scoped to the selected connection.
4. Switching connections restores the previous cursor position.
5. Connection bar is hidden when only one connection is configured (no regression
   for single-connection users).
6. `u` / `d` / `b` keys act on the currently selected connection.
7. Live stats update for **all** connections simultaneously (the daemon poll
   dispatches `UpdatePeers` with all connections' statuses).
8. `cargo test --workspace --all-features`, `cargo clippy --all-targets
   --all-features -- -D warnings -D clippy::pedantic`, and `cargo fmt --check`
   all pass.
