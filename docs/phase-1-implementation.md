# Phase 1: Live Daemon Integration — Implementation Plan

## Context

The TUI is now cleanly structured across three crates (Phase 0 complete) but
is still **static** — it loads config at startup and never talks to the daemon.
Phase 1 makes it live: periodic status polling, up/down actions, backend
switching, and graceful degradation when the daemon is unreachable.

**Done when:** TUI can reliably bring up/down any connection and reflect daemon
state within < 1 s. Daemon disconnect is detected and displayed within 2 ticks.
All daemon commands round-trip without blocking the UI.

## Architecture

The daemon uses one-shot Unix socket connections (connect → send command →
read response → close). The IPC client currently lives in `ferro-wg/src/client.rs`.
The TUI gets its own copy to avoid coupling `ferro-wg-core` to the client pattern.

```
Event Loop (ferro-wg-tui)
  │
  ├─ AppEvent::Tick (250ms)
  │    └─ tokio::spawn → send_command(Status) → mpsc::send(DaemonMessage)
  │
  ├─ Drain mpsc channel (non-blocking)
  │    ├─ DaemonMessage::StatusUpdate(Vec<PeerStatus>) → Action::UpdatePeers
  │    ├─ DaemonMessage::CommandOk(String)              → Action::DaemonOk
  │    └─ DaemonMessage::CommandError(String)            → Action::DaemonError
  │
  ├─ AppEvent::Key → handle_global_key / component.handle_key
  │    ├─ Enter or 'u' → Action::ConnectPeer (bring up selected)
  │    ├─ 'd'          → Action::DisconnectPeer (tear down selected)
  │    └─ 'b'          → Action::CyclePeerBackend (cycle backend)
  │
  └─ Render (components read from updated AppState)
```

### Why async spawns, not blocking in Tick

The daemon round-trip is ~1-5 ms locally but could be slower under load.
Spawning a `tokio::spawn` task for each status poll keeps the UI responsive.
Results flow back via an `mpsc` channel that the event loop drains each frame.

## Dependency Graph

```
ferro-wg-core  ← unchanged (IPC types only)
    ↑
ferro-wg-tui-core  ← new Action variants, AppState daemon fields
    ↑
ferro-wg-tui-components  ← up/down keybinds, feedback display
    ↑
ferro-wg-tui  ← daemon client (copied), poller, mpsc channel, event loop drain
```

## Implementation Steps

### Step 1: Add IPC client to `ferro-wg-tui` (~70 lines)

**Create:** `ferro-wg-tui/src/client.rs` — copied from `ferro-wg/src/client.rs`

The TUI gets its own copy of the daemon client. This keeps `ferro-wg-core`
focused on types/traits and avoids coupling the core crate to the client
pattern. The binary's `ferro-wg/src/client.rs` stays unchanged.

**Tests:** Existing workspace tests pass. Client compiles in new location.

### Step 2: Add daemon Action variants and AppState fields (~120 lines)

**Modify:** `ferro-wg-tui-core/src/action.rs`

New variants:
```rust
/// Update peer state from daemon status response.
UpdatePeers(Vec<PeerStatus>),
/// Bring up the selected connection.
ConnectPeer(String),
/// Tear down the selected connection.
DisconnectPeer(String),
/// Cycle the backend for the selected connection.
CyclePeerBackend(String),
/// Daemon returned an error.
DaemonError(String),
/// Daemon command succeeded with a message.
DaemonOk(String),
/// Daemon connectivity changed.
DaemonConnectivityChanged(bool),
```

**Modify:** `ferro-wg-tui-core/src/state.rs`

New AppState fields:
```rust
/// Whether the daemon is reachable.
pub daemon_connected: bool,
/// Transient feedback message (success or error) with expiry.
pub feedback: Option<Feedback>,
```

New struct:
```rust
pub struct Feedback {
    pub message: String,
    pub is_error: bool,
    pub expires_at: Instant,
}
```

Handle `UpdatePeers` in dispatch:
- Match each `PeerStatus.name` against `peers[*].config.name`
- Update `connected`, `stats`, `backend` for matching peers
- Set `daemon_connected = true`

Handle `DaemonConnectivityChanged(connected)`:
- Set `daemon_connected`

Handle `DaemonError(msg)` / `DaemonOk(msg)`:
- Set `feedback` with appropriate `is_error` and 3 s expiry

Handle `ConnectPeer`, `DisconnectPeer`, `CyclePeerBackend`:
- No-op in AppState (these trigger daemon commands in the event loop,
  not direct state changes)

**Tests:** dispatch `UpdatePeers` updates peer state. Feedback expires.
Connectivity flag tracks.

### Step 3: Add daemon poller to event loop (~150 lines)

**Modify:** `ferro-wg-tui/src/lib.rs`

Add a `DaemonMessage` enum and mpsc channel:
```rust
enum DaemonMessage {
    StatusUpdate(Vec<PeerStatus>),
    CommandOk(String),
    CommandError(String),
    Unreachable,
}
```

In `event_loop()`:
1. Create `mpsc::unbounded_channel::<DaemonMessage>()`
2. On `AppEvent::Tick`: spawn status poll task
3. Before rendering: drain channel with `try_recv()` loop, dispatch Actions
4. On `ConnectPeer`/`DisconnectPeer`/`CyclePeerBackend` Actions: spawn command task
5. Clear expired feedback on each tick

Status poll task:
```rust
tokio::spawn(async move {
    match client::send_command(&DaemonCommand::Status).await {
        Ok(DaemonResponse::Status(peers)) => {
            let _ = tx.send(DaemonMessage::StatusUpdate(peers));
        }
        Err(_) => {
            let _ = tx.send(DaemonMessage::Unreachable);
        }
        _ => {}
    }
});
```

Command task (for up/down/switch):
```rust
tokio::spawn(async move {
    match client::send_command(&cmd).await {
        Ok(DaemonResponse::Ok) => {
            let _ = tx.send(DaemonMessage::CommandOk(msg));
        }
        Ok(DaemonResponse::Error(e)) => {
            let _ = tx.send(DaemonMessage::CommandError(e));
        }
        Err(e) => {
            let _ = tx.send(DaemonMessage::CommandError(e));
        }
        _ => {}
    }
});
```

Throttle: only spawn status poll if no poll is in-flight (use `Arc<AtomicBool>`
or simply skip tick if last poll hasn't returned yet).

**Tests:** Unit test for channel drain → Action dispatch mapping.

### Step 4: Add keybinds for up/down/backend to StatusComponent (~60 lines)

**Modify:** `ferro-wg-tui-components/src/status.rs`

In `handle_key()`, add:
```rust
KeyCode::Enter | KeyCode::Char('u') => {
    // Get selected peer name from table state index
    Some(Action::ConnectPeer(peer_name))
}
KeyCode::Char('d') => Some(Action::DisconnectPeer(peer_name)),
KeyCode::Char('b') => Some(Action::CyclePeerBackend(peer_name)),
```

The component owns `TableState` with the selected index. Use
`state.filtered_peers().nth(selected_index)` to resolve the peer name.

**Tests:** `handle_key` returns correct Action with peer name for u/d/b keys.

### Step 5: Update StatusBarComponent to show daemon status and feedback (~50 lines)

**Modify:** `ferro-wg-tui-components/src/status_bar.rs`

In `render()` Normal mode, prepend daemon status indicator:
```
[●] connected  |  q quit  / search  1-5 tabs  j/k navigate  u up  d down  b backend
[○] offline    |  q quit  / search  ...
```

Show feedback message when `state.feedback` is `Some` and not expired:
```
[●] ✓ Brought up: dc-mia          (green, fades after 3s)
[●] ✗ error: connection not found  (red, fades after 3s)
```

**Tests:** Render outputs correct indicator based on `daemon_connected`.

### Step 6: Update help text and keybind documentation (~30 lines)

**Modify:** `ferro-wg-tui-components/src/status_bar.rs` — add u/d/b to help line
**Modify:** Status/Peers component doc comments

## Verification

```bash
cargo test --workspace --features boringtun,neptun,gotatun
cargo build --workspace
cargo clippy --workspace --features boringtun,neptun,gotatun -- -W clippy::pedantic -D warnings
cargo fmt --all --check
```

Integration test (manual):
1. Start daemon: `sudo ferro-wg daemon`
2. Import a config: `ferro-wg import test.conf`
3. Run TUI: `cargo run --features tui -- tui`
4. Verify status updates appear within 1 s
5. Press `u` to bring up a connection — verify feedback
6. Press `d` to tear down — verify feedback
7. Stop daemon — verify offline indicator within 500 ms
8. Restart daemon — verify reconnection

## Critical Files

| File | Changes |
|---|---|
| `ferro-wg-tui/src/client.rs` | NEW (copied from `ferro-wg/src/client.rs`) |
| `ferro-wg-tui-core/src/action.rs` | Add 7 new Action variants |
| `ferro-wg-tui-core/src/state.rs` | Add daemon fields, `UpdatePeers` dispatch, `Feedback` |
| `ferro-wg-tui/src/lib.rs` | Daemon poller, mpsc channel, command dispatch |
| `ferro-wg-tui-components/src/status.rs` | Up/down/backend keybinds |
| `ferro-wg-tui-components/src/status_bar.rs` | Daemon indicator, feedback display, help text |
