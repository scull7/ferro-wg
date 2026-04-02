# Phase 0: Extract TUI into Three Crates + Component Architecture

## Context

The ferro-wg TUI (~911 LoC, 4 files) is monolithic: a single `App` struct holds
all state, key handling is a big match block, and rendering dispatches through
free functions. This blocks Phase 1+ — daemon integration, multi-connection,
and log streaming all need isolated component state and async action dispatch.

Phase 0 restructures to a [tursotui](https://github.com/mikeleppane/tursotui)-inspired
component architecture **and** extracts the TUI into three dedicated crates.
The TUI currently has zero cross-dependencies with CLI/client code and depends
on only 5 types from `ferro-wg-core`, making extraction clean.

**Goal:** Identical visual output and keybindings after refactor. No new features.

## Crate Architecture

```
ferro-wg-core               (existing — unchanged)
    ↑
ferro-wg-tui-core            (Component trait, Action, AppState, Theme)
    ↑
ferro-wg-tui-components      (StatusComponent, PeersComponent, etc.)
    ↑
ferro-wg-tui                 (event loop, terminal, wiring)
    ↑
ferro-wg                     (binary — CLI + optional TUI)
```

**Key decisions:**

| Decision | Choice | Why |
|---|---|---|
| Crate split | Three TUI crates (core, components, shell) | Maximum separation; core types reusable, components independently testable |
| Component storage | `Vec<Box<dyn Component>>` separate from `AppState` | Avoids `&mut component` + `&state` split-borrow |
| Shared data | Components receive `&AppState` (read-only) | Single source of truth, no data duplication |
| Table state | Per-component | Independent scroll positions per tab |
| Search query | Lives in `AppState`; components call `state.filtered_peers()` | Search affects Status + Peers uniformly |
| Theme | `AppState.theme`; passed via `&AppState` | Single instance, consistent styling |
| Key routing | Search mode → `StatusBarComponent`; Normal → global keys first, then active component | Matches existing behavior exactly |

## Target Structure

```
ferro-wg-tui-core/
  Cargo.toml
  src/
    lib.rs             ← re-exports
    action.rs          ← Action enum
    state.rs           ← AppState, PeerState, dispatch()
    theme.rs           ← Theme (Catppuccin Mocha/Latte)
    component.rs       ← Component trait, panel_block(), overlay_block()
    app.rs             ← Tab, InputMode enums
    util.rs            ← format_bytes()

ferro-wg-tui-components/
  Cargo.toml
  src/
    lib.rs             ← re-exports
    status.rs          ← StatusComponent
    peers.rs           ← PeersComponent
    compare.rs         ← CompareComponent
    config.rs          ← ConfigComponent
    logs.rs            ← LogsComponent
    tab_bar.rs         ← TabBarComponent
    status_bar.rs      ← StatusBarComponent

ferro-wg-tui/
  Cargo.toml
  src/
    lib.rs             ← pub async fn run(), event loop, terminal setup
    event.rs           ← EventHandler (crossterm polling)
```

## Cargo.toml Specifications

### ferro-wg-tui-core/Cargo.toml

```toml
[package]
name = "ferro-wg-tui-core"
edition.workspace = true
version.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true
description = "Core types for the ferro-wg TUI: Component trait, Action, AppState, Theme"

[dependencies]
ferro-wg-core = { path = "../ferro-wg-core" }
ratatui = "0.29"
crossterm = "0.28"

[lints]
workspace = true
```

### ferro-wg-tui-components/Cargo.toml

```toml
[package]
name = "ferro-wg-tui-components"
edition.workspace = true
version.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true
description = "TUI components for ferro-wg: Status, Peers, Compare, Config, Logs"

[dependencies]
ferro-wg-tui-core = { path = "../ferro-wg-tui-core" }
ferro-wg-core = { path = "../ferro-wg-core" }
ratatui = "0.29"
crossterm = "0.28"

[lints]
workspace = true
```

### ferro-wg-tui/Cargo.toml

```toml
[package]
name = "ferro-wg-tui"
edition.workspace = true
version.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true
description = "Terminal UI for ferro-wg: event loop, terminal management, component wiring"

[dependencies]
ferro-wg-tui-core = { path = "../ferro-wg-tui-core" }
ferro-wg-tui-components = { path = "../ferro-wg-tui-components" }
ferro-wg-core = { path = "../ferro-wg-core" }
ratatui = "0.29"
crossterm = "0.28"
tokio = { version = "1", features = ["full"] }

[lints]
workspace = true
```

### ferro-wg/Cargo.toml changes

```toml
[features]
tui = ["dep:ferro-wg-tui"]
# Remove: dep:ratatui, dep:crossterm

[dependencies]
ferro-wg-tui = { path = "../ferro-wg-tui", optional = true }
# Remove: ratatui, crossterm
```

### Workspace Cargo.toml

```toml
[workspace]
members = [
    "ferro-wg-core",
    "ferro-wg-tui-core",
    "ferro-wg-tui-components",
    "ferro-wg-tui",
    "ferro-wg",
    "ferro-wg-daemon",
]
```

## Event Loop Architecture

```
Event Loop (ferro-wg-tui/src/lib.rs)
  │
  ├─ crossterm key → handle_global_key() or active_component.handle_key()
  │                    returns Option<Action>
  │
  ├─ Action → state.dispatch(action)      ← Phase 1: state mutation
  │         → component.update(&action)   ← Phase 2: component notification
  │
  └─ Render: tab_bar.render(), components[active].render(), status_bar.render()
             each receives &AppState for shared data
```

```rust
// ferro-wg-tui/src/lib.rs
pub async fn run(wg_config: WgConfig) -> Result<(), Box<dyn std::error::Error>> {
    // Terminal setup...

    let mut state = AppState::new(wg_config);
    let mut components: Vec<Box<dyn Component>> = vec![
        Box::new(StatusComponent::new()),
        Box::new(PeersComponent::new()),
        Box::new(CompareComponent::new()),
        Box::new(ConfigComponent::new()),
        Box::new(LogsComponent::new()),
    ];
    let mut tab_bar = TabBarComponent::new();
    let mut status_bar = StatusBarComponent::new();

    while state.running {
        terminal.draw(|frame| {
            let chunks = layout(frame.area());
            tab_bar.render(frame, chunks[0], false, &state);
            components[state.active_tab.index()].render(frame, chunks[1], true, &state);
            status_bar.render(frame, chunks[2], false, &state);
        })?;

        match events.next().await {
            Some(AppEvent::Key(key)) => {
                let action = if state.input_mode == InputMode::Search {
                    status_bar.handle_key(key, &state)
                } else {
                    handle_global_key(key)
                        .or_else(|| components[state.active_tab.index()].handle_key(key, &state))
                };
                if let Some(ref action) = action {
                    state.dispatch(action);
                    for comp in &mut components { comp.update(action, &state); }
                }
            }
            Some(AppEvent::Tick) => { /* future: daemon poll */ }
            None => break,
        }
    }

    // Terminal teardown...
}
```

## Core Type Definitions

### Component Trait (ferro-wg-tui-core)

```rust
pub trait Component {
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action>;
    fn update(&mut self, action: &Action, state: &AppState);
    fn render(&mut self, frame: &mut Frame, area: Rect, focused: bool, state: &AppState);
}
```

### Action Enum (ferro-wg-tui-core)

```rust
pub enum Action {
    Quit,
    NextTab,
    PrevTab,
    SelectTab(Tab),
    NextRow,
    PrevRow,
    EnterSearch,
    ExitSearch,
    ClearSearch,
    SearchInput(char),
    SearchBackspace,
    Tick,
}
```

## Implementation Steps

### Step 1: Create `ferro-wg-tui-core` (~350 lines)

**Create:** entire `ferro-wg-tui-core/` crate

Contains: `Action` enum, `Tab`/`InputMode` enums (from `app.rs`), `Component`
trait + `panel_block()`/`overlay_block()`, `Theme` struct with `mocha()`/`latte()`,
`AppState` + `PeerState` + `dispatch()` + `filtered_peers()` (from `App`),
`format_bytes()` utility.

Phase 0 theme maps to identical Color values currently hardcoded (Cyan, Green,
DarkGray). True Catppuccin hex values swap in during Phase 7.

**Tests:** Tab enum tests, AppState dispatch tests, Theme color assertions,
format_bytes test. All migrated from existing `app.rs` and `ui.rs` tests.

### Step 2: Create `ferro-wg-tui-components` (~500 lines)

**Create:** entire `ferro-wg-tui-components/` crate

Seven components, each implementing `Component`:

- **StatusComponent** — owns `TableState`, renders peer status table (Peer, Endpoint, Status, Rx, Tx, Handshake)
- **PeersComponent** — owns `TableState`, renders peer config table (Peer, PubKey, Endpoint, AllowedIPs, Keepalive, Backend)
- **CompareComponent** — owns `TableState`, renders backend comparison (fixed 3-row; fixes latent bug where `peers.len()` was used for row count)
- **ConfigComponent** — no table state, renders interface config Paragraph
- **LogsComponent** — no table state, renders log lines Paragraph
- **TabBarComponent** — render-only, tab switching handled globally
- **StatusBarComponent** — in Search mode emits `SearchInput`/`SearchBackspace`/`ExitSearch`; in Normal mode renders help text

**Tests:** `handle_key` → Action mapping, row clamping, search action emission, render-doesn't-panic with empty state.

### Step 3: Create `ferro-wg-tui` + rewire binary (~220 lines)

**Create:** `ferro-wg-tui/` crate with `event.rs` (from existing) and `lib.rs` (event loop + terminal setup/teardown)

**Modify:**
- `Cargo.toml` (workspace) — add 3 new members
- `ferro-wg/Cargo.toml` — replace ratatui/crossterm deps with `ferro-wg-tui`
- `ferro-wg/src/main.rs` — `use ferro_wg_tui::run` instead of `mod tui`

**Delete:** `ferro-wg/src/tui/` directory (all 4 files)

**Tests:** Full workspace `cargo test`, `cargo build`, `cargo clippy`, `cargo fmt`.

## Verification

After all steps:

```bash
cargo test --workspace --features boringtun,neptun,gotatun
cargo build --workspace
cargo clippy --workspace --features boringtun,neptun,gotatun -- -W clippy::pedantic -D warnings
cargo fmt --all --check
```

Individual crate builds:
```bash
cargo build -p ferro-wg-tui-core         # core types only
cargo build -p ferro-wg-tui-components   # components + core
cargo build -p ferro-wg-tui              # full TUI
cargo build -p ferro-wg --no-default-features  # CLI-only, no TUI
```

- Visual output identical to current TUI
- All keybindings preserved: q, Esc, Ctrl+C, Tab, Shift+Tab, Left/Right, 1-5, j/k, Up/Down, /
- Search filters peers in Status and Peers tabs
- New component added by: implementing `Component` in `ferro-wg-tui-components`, adding `Tab` variant, pushing to component `Vec`

## File Migration Map

| Current file | Destination |
|---|---|
| `ferro-wg/src/tui/app.rs` — Tab, InputMode | `ferro-wg-tui-core/src/app.rs` |
| `ferro-wg/src/tui/app.rs` — PeerState, App | `ferro-wg-tui-core/src/state.rs` |
| `ferro-wg/src/tui/ui.rs` — draw_status_view | `ferro-wg-tui-components/src/status.rs` |
| `ferro-wg/src/tui/ui.rs` — draw_peers_view | `ferro-wg-tui-components/src/peers.rs` |
| `ferro-wg/src/tui/ui.rs` — draw_compare_view | `ferro-wg-tui-components/src/compare.rs` |
| `ferro-wg/src/tui/ui.rs` — draw_config_view | `ferro-wg-tui-components/src/config.rs` |
| `ferro-wg/src/tui/ui.rs` — draw_logs_view | `ferro-wg-tui-components/src/logs.rs` |
| `ferro-wg/src/tui/ui.rs` — draw_tabs | `ferro-wg-tui-components/src/tab_bar.rs` |
| `ferro-wg/src/tui/ui.rs` — draw_status_bar | `ferro-wg-tui-components/src/status_bar.rs` |
| `ferro-wg/src/tui/ui.rs` — format_bytes | `ferro-wg-tui-core/src/util.rs` |
| `ferro-wg/src/tui/mod.rs` — run, event_loop | `ferro-wg-tui/src/lib.rs` |
| `ferro-wg/src/tui/event.rs` — EventHandler | `ferro-wg-tui/src/event.rs` |
| `ferro-wg/src/main.rs` — run_tui() | Modified in place |
| `Cargo.toml` (workspace) | Add 3 new members |
