# Phase 0: TUI Component Architecture Refactor

## Context

The ferro-wg TUI (~911 LoC, 4 files) is monolithic: a single `App` struct holds
all state, key handling is a big match block, and rendering dispatches through
free functions. This works but blocks Phase 1+ — daemon integration, multi-connection,
and log streaming all need isolated component state and async action dispatch.

Phase 0 restructures to a [tursotui](https://github.com/mikeleppane/tursotui)-inspired
component architecture: `Component` trait, `Action` enum, centralized `AppState`,
and per-tab components with their own `TableState`.

**Goal:** Identical visual output and keybindings after refactor. No new features.

## Architecture

```
Event Loop (mod.rs)
  │
  ├─ crossterm key → handle_global_key() or active_component.handle_key()
  │                    returns Option<Action>
  │
  ├─ Action → state.dispatch(action)      ← Phase 1: state mutation
  │         → component.update(&action)   ← Phase 2: component notification
  │
  └─ Render: tab_bar.render(), components[active].render(), status_bar.render()
             each receives &AppState for shared data, &Theme for styling
```

**Key decisions:**

| Decision | Choice | Why |
|---|---|---|
| Component storage | `Vec<Box<dyn Component>>` separate from `AppState` | Avoids `&mut component` + `&state` split-borrow |
| Shared data | Components receive `&AppState` (read-only) | Single source of truth, no data duplication |
| Table state | Per-component | Independent scroll positions per tab |
| Search query | Lives in `AppState`; components call `state.filtered_peers()` | Search affects Status + Peers uniformly |
| Theme | `AppState.theme`; passed via `&AppState` | Single instance, consistent styling |
| Key routing | Search mode → `StatusBarComponent`; Normal → global keys first, then active component | Matches existing behavior exactly |

## Target File Structure

```
ferro-wg/src/tui/
  mod.rs              ← event loop rewritten to use components
  event.rs            ← unchanged
  action.rs           ← NEW: Action enum
  state.rs            ← NEW: AppState (from App) + dispatch()
  theme.rs            ← NEW: Theme with Catppuccin Mocha/Latte
  component.rs        ← NEW: Component trait + panel_block/overlay_block helpers
  components/
    mod.rs            ← re-exports
    status.rs         ← StatusComponent (from draw_status_view)
    peers.rs          ← PeersComponent (from draw_peers_view)
    compare.rs        ← CompareComponent (from draw_compare_view)
    config.rs         ← ConfigComponent (from draw_config_view)
    logs.rs           ← LogsComponent (from draw_logs_view)
    tab_bar.rs        ← TabBarComponent (from draw_tabs)
    status_bar.rs     ← StatusBarComponent (from draw_status_bar)
  app.rs              ← slim: Tab, InputMode, PeerState only
  ui.rs               ← slim: format_bytes utility only
```

## Implementation Steps

### Step 1: `Action` enum + `Component` trait (~90 lines)

**Create:** `action.rs`, `component.rs`
**Modify:** `mod.rs` (add module declarations)

```rust
// action.rs
pub enum Action {
    Quit, NextTab, PrevTab, SelectTab(Tab),
    NextRow, PrevRow,
    EnterSearch, ExitSearch, ClearSearch,
    SearchInput(char), SearchBackspace, Tick,
}

// component.rs
pub trait Component {
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action>;
    fn update(&mut self, action: &Action, state: &AppState);
    fn render(&mut self, frame: &mut Frame, area: Rect, focused: bool, state: &AppState);
}

pub fn panel_block(title: &str, theme: &Theme) -> Block { ... }
pub fn overlay_block(title: &str, theme: &Theme) -> Block { ... }
```

**Tests:** Compilation only — pure type definitions.

### Step 2: `Theme` module (~140 lines)

**Create:** `theme.rs`

```rust
pub struct Theme {
    pub base, surface, text, subtext, accent, success, error, warning, muted, highlight_bg: Color,
}
impl Theme {
    pub fn mocha() -> Self { /* maps to current hardcoded colors */ }
    pub fn latte() -> Self { ... }
    pub fn header_style(&self) -> Style { ... }
    pub fn highlight_style(&self) -> Style { ... }
}
```

Phase 0 maps to identical Color values (Cyan, Green, DarkGray, etc.).
True Catppuccin hex values swap in during Phase 7.

**Tests:** Assert `Theme::mocha()` colors match current hardcoded values.

### Step 3: Extract `AppState` from `App` (~200 lines)

**Create:** `state.rs`
**Modify:** `app.rs` — `App` wraps `AppState` internally so existing tests pass unchanged

`AppState` contains: `running`, `active_tab`, `input_mode`, `search_query`,
`wg_config`, `peers`, `log_lines`, `theme`. Plus `dispatch(&mut self, action)`
and `filtered_peers()`.

`App` becomes a thin wrapper delegating to `AppState`. All 11 existing `app.rs`
tests pass without modification.

**Tests:** Existing tests pass. New tests for `AppState::dispatch` (Quit, tab nav, search).

### Step 4: `StatusComponent` + `PeersComponent` (~240 lines)

**Create:** `components/mod.rs`, `components/status.rs`, `components/peers.rs`

Each component:
- Owns its own `TableState`
- `render()` contains logic extracted from `draw_status_view` / `draw_peers_view`
- `handle_key()` returns `Action::NextRow` / `Action::PrevRow` for j/k/arrows
- `update()` handles row clamping and resets selection on tab change
- Uses `state.filtered_peers()` and `theme.header_style()`

**Tests:** `handle_key` returns correct Actions. `update` resets selection. Row clamping.

### Step 5: `CompareComponent`, `ConfigComponent`, `LogsComponent` (~200 lines)

**Create:** `components/compare.rs`, `components/config.rs`, `components/logs.rs`

- Compare: fixed 3-row backend table (fixes latent bug — currently uses `peers.len()`)
- Config: Paragraph rendering, no table state
- Logs: Paragraph rendering, no table state (future: scroll)

**Tests:** Compare row clamping (3 rows). Render doesn't panic with empty state.

### Step 6: `TabBarComponent` + `StatusBarComponent` (~120 lines)

**Create:** `components/tab_bar.rs`, `components/status_bar.rs`

- TabBar: render-only (tab switching handled globally)
- StatusBar: in Search mode, emits `SearchInput(char)` / `SearchBackspace` / `ExitSearch`; in Normal mode, render-only help text

**Tests:** StatusBarComponent emits correct search actions.

### Step 7: Wire components into event loop (~200 lines)

**Modify:** `mod.rs` (rewrite `event_loop`), `ui.rs` (simplify or inline `draw`)

```rust
async fn event_loop(...) {
    let mut state = AppState::new(wg_config);
    let mut components: Vec<Box<dyn Component>> = vec![...];
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
}
```

**Tests:** Compilation + existing `format_bytes` test. Manual visual verification.

### Step 8: Remove legacy `App`, migrate tests, clean up (~200 lines, mostly deletions)

**Modify:** `app.rs` — remove `App` struct, keep `Tab`, `InputMode`, `PeerState`

- Tab tests stay in `app.rs`
- State/dispatch/filter tests move to `state.rs`
- Row navigation tests move to component tests
- `ui.rs` retains only `format_bytes` as `pub(crate)`
- Doc comments on all public items
- `cargo clippy` clean, `cargo fmt` clean

**Tests:** All original tests pass in new locations. Total test count higher than the original 12.

## Verification

After all 8 steps:

```bash
cargo test --workspace --features boringtun,neptun,gotatun
cargo build --workspace
cargo clippy --workspace --features boringtun,neptun,gotatun -- -W clippy::pedantic -D warnings
cargo fmt --all --check
```

- Visual output identical (run `cargo run --features tui -- tui` and compare)
- All keybindings work: q, Esc, Ctrl+C, Tab, Shift+Tab, Left/Right, 1-5, j/k, Up/Down, /
- Search filters peers in Status and Peers tabs
- New component can be added by: implementing `Component`, adding a `Tab` variant, pushing to the `Vec`

## Critical Files

| File | Role |
|---|---|
| `ferro-wg/src/tui/app.rs` | Source of Tab/InputMode/PeerState + all current tests |
| `ferro-wg/src/tui/ui.rs` | Source of all rendering logic to extract |
| `ferro-wg/src/tui/mod.rs` | Event loop to rewrite |
| `ferro-wg/src/tui/event.rs` | EventHandler — unchanged but critical to wiring |
| `ferro-wg/Cargo.toml` | ratatui 0.29, crossterm 0.28 deps |
| `Cargo.toml` (workspace) | Already has `unsafe_code = "forbid"` |
