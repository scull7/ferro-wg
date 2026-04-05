# Phase 7: UX Polish — Implementation Plan

## Pre-implementation checklist

Before writing any Rust code, invoke `/rust-code-writer` (per `CLAUDE.md`).

- Confirm `Theme::mocha()` and `Theme::latte()` currently use placeholder `Color::*` terminal
  constants (`base`, `surface`, and `text` all map to `Color::Reset`). Phase 7 replaces all
  ten fields in both constructors with `Color::Rgb(r, g, b)` values drawn from the official
  Catppuccin palette. No new `Theme` fields are needed; all existing consumers
  (`header_style`, `highlight_style`, `hotkey_style`, etc.) automatically pick up the new
  colors.
- Confirm `Color::Rgb(u8, u8, u8)` is the correct ratatui variant for 24-bit color — it is
  available in `ratatui::style::Color` at the project's locked version (0.29). Do **not** use
  `Color::from_u32`; that function does not exist in ratatui 0.29.
- Confirm that adding a `toggle_theme` field to `AppState` requires a new `ThemeKind` enum in
  `ferro-wg-tui-core/src/theme.rs` and a new `Action::ToggleTheme` variant — both are trivially
  addable without touching any other module. `AppState::theme` becomes a `Theme` field that is
  swapped on `ToggleTheme`.
- Confirm `centered_rect` currently lives as a private function in
  `ferro-wg-tui-components/src/confirm_dialog.rs`. Phase 7 extracts it to a new
  `pub(crate) fn centered_rect` in `ferro-wg-tui-components/src/util.rs` (as Phase 6 plans). If
  Phase 6 has already landed this extraction, skip the extraction step in US-3 (Help overlay) and
  US-4 (Toast component) and import directly from `crate::util`. If Phase 6 has **not** landed,
  the US-3 commit must perform the extraction.
- Confirm the `Component` trait in `ferro-wg-tui-core/src/component.rs` only has `handle_key`,
  `update`, and `render`. Mouse support requires either a new default `fn handle_mouse` method on
  the trait (additive — all existing implementors inherit a `None`-returning default) or a
  separate `MouseHandler` trait. The plan uses a default method so no component is forced to opt
  in.
- Confirm `AppEvent` in `ferro-wg-tui/src/event.rs` currently discards `Event::Mouse` events.
  Adding mouse support requires: (1) enabling crossterm `MouseCapture` mode in `run()` alongside
  `EnterAlternateScreen`, (2) adding `AppEvent::Mouse(MouseEvent)` to the enum, (3) forwarding
  `Event::Mouse(m)` in the `EventHandler::run` loop, (4) routing `AppEvent::Mouse` in the
  event-loop match arm.
- Confirm `AppState::feedback: Option<Feedback>` is the single-slot feedback type. Phase 7
  replaces it with `toasts: VecDeque<Toast>`. Multiple async events (e.g. handshake completed +
  peer down) can arrive simultaneously and must not overwrite each other. Existing
  `Feedback::success` / `Feedback::error` call sites are updated to `push_toast(Toast::success(...))`
  in the same commit.
- Confirm `insta` is **not** yet in any crate's `[dev-dependencies]`. Each crate that gains
  snapshot tests in Phase 7 adds `insta = { version = "1", features = ["yaml"] }` to its
  `[dev-dependencies]`. Snapshots live in `<crate>/src/snapshots/`.
- Confirm `thiserror` is already a `[dependencies]` entry in `ferro-wg-tui-components` and
  `ferro-wg-tui-core`. Phase 7 introduces no new error types (all UX failures surface as
  `Action::DaemonError` feedback messages).
- Confirm crossterm 0.28 supports `EnableMouseCapture` / `DisableMouseCapture` as execute macros.
  Mouse event types live in `crossterm::event::{MouseEvent, MouseEventKind, MouseButton}`.
- Confirm the `?` key is currently unbound globally (not mapped in `handle_global_key`). Adding
  `Action::ShowHelp` / `Action::HideHelp` introduces no conflict.
- Confirm `T` (uppercase) is free in all tab contexts. The plan binds `T` to `ToggleTheme`
  globally because lowercase `t` is also free but uppercase distinguishes it from future
  tab-local bindings.

---

## Context

Phase 6 delivered interactive config editing: users can navigate the Config tab, edit interface
and peer fields with inline validation, preview a unified diff before saving, and apply changes
with an automatic `.bak` backup. As a by-product of Phase 6 the `util.rs` module in
`ferro-wg-tui-components` gained `pub(crate) centered_rect` and `DiffPreviewComponent` was added
to `ComponentBundle`.

Phase 7 polishes the entire TUI surface. No new *functional* capability is added — the focus is
entirely on user experience: the theme fills in real Catppuccin hex values, the layout degrades
gracefully on 80×24 terminals, mouse clicks navigate tabs and rows, a help overlay surfaces all
keybindings on demand, and async events surface as non-blocking toast notifications rather than
single-slot feedback messages.

**What is missing entering Phase 7:**

| Feature | Current state | Missing |
|---------|--------------|---------|
| Catppuccin colors | Both `mocha()` and `latte()` use `Color::Reset` / terminal placeholders | Real 24-bit hex values for all 10 semantic roles in both palettes |
| Theme toggle | `AppState::theme` is hardcoded to `Theme::mocha()` in `AppState::new()` | `ThemeKind` enum, `Action::ToggleTheme`, keybinding `T` |
| Responsive layout | No minimum-size guard; content area can receive 0-height rect | Explicit 80×24 enforcement; `too-small` fallback render |
| Mouse support | `EventHandler` discards `Event::Mouse(_)` entirely | `AppEvent::Mouse`, `MouseCapture` mode, tab-click and scroll routing |
| Help overlay | `?` key is unmapped; no help display | `HelpOverlayComponent`, `Action::ShowHelp`/`HideHelp`, `AppState::show_help: bool` |
| Notification toasts | Single-slot `feedback: Option<Feedback>` — new feedback overwrites previous | Replace with `toasts: VecDeque<Toast>` queue; dedicated `ToastComponent` |
| Snapshot tests | None | `insta`-based snapshot tests for each component at 80×24 and 120×40 |

**Done when:** every component renders correctly at 80×24 and 120×40, both Catppuccin Mocha and
Latte palettes apply visually, clicking a tab header navigates to that tab, scrolling with the
mouse wheel moves row selection, pressing `?` shows the full keybinding overlay, pressing `T`
switches between Mocha and Latte, and multiple async toast notifications queue and expire
independently.

---

## User Stories

| ID | User story | Acceptance criteria |
|----|------------|---------------------|
| US-1 | As a user I want the Catppuccin Mocha and Latte themes fully applied | Both `Theme::mocha()` and `Theme::latte()` use `Color::Rgb` values from the official palette; pressing `T` switches between them; all 10 semantic roles are non-placeholder in both themes |
| US-2 | As a user I want the TUI to not crash or produce garbage at 80×24 | At exactly 80×24: all six tabs render without panic; the connection bar is suppressed when height is insufficient; the status and tab bars are always visible; content area height is always ≥ 1 |
| US-3 | As a user I want a help overlay when I press `?` | Pressing `?` in any tab opens a full-screen semi-transparent overlay listing all keybindings in two columns; pressing `?`, `Esc`, or `q` closes it; the overlay is the topmost render layer; `AppState::show_help: bool` guards it |
| US-4 | As a user I want notification toasts for async events | Multiple `DaemonOk`/`DaemonError` messages queue independently (up to 5 visible); each toast expires after 3 s; toasts render in the bottom-right corner above the status bar without covering content; a new toast does not evict an unexpired one |
| US-5 | As a user I want to click tabs to navigate | A left mouse click whose `y` falls within the tab bar row and whose `x` maps to a tab header navigates to that tab |
| US-6 | As a user I want to scroll with the mouse to move row selection | `MouseEventKind::ScrollDown` dispatches `Action::NextRow`; `ScrollUp` dispatches `Action::PrevRow`; mouse scroll is ignored when any overlay is open |

---

## Architecture

### Existing infrastructure to reuse

```
Theme::mocha() / Theme::latte()           ← replace placeholder colors in-place (theme.rs)
AppState::theme: Theme                    ← add ThemeKind field for toggle (state.rs)
AppState::feedback: Option<Feedback>      ← replace with toasts: VecDeque<Toast> (state.rs)
AppState::show_help: bool                 ← new field, initialized false (state.rs)
ConfirmDialogComponent pattern            ← model HelpOverlayComponent + ToastComponent
centered_rect(pct_x, height, area)        ← pub(crate) in util.rs (from Phase 6, or extract here)
compute_layout() in lib.rs                ← extend with 80×24 guard
handle_global_key() in lib.rs             ← add ? → ShowHelp, T → ToggleTheme
ComponentBundle in lib.rs                 ← add help_overlay + toast components
render_ui() in lib.rs                     ← render help_overlay topmost; toast above status
EventHandler in event.rs                  ← add AppEvent::Mouse, enable MouseCapture
handle_key_event routing chain in lib.rs  ← add HelpOverlay guard; add mouse routing
dispatch_all() in lib.rs                  ← update for help_overlay and toast components
```

### Stratified layer design

Three layers, unchanged from Phases 5 and 6:

1. **Calculation layer** — `ferro-wg-tui-core/src/theme.rs` (Catppuccin palette constants),
   `ferro-wg-tui-core/src/ux.rs` (new module: toast expiry logic, keybinding table, mouse
   hit-test)
2. **State layer** — `ferro-wg-tui-core/src/state.rs` (`ThemeKind`, `Toast`, dispatch arms for
   new actions, `show_help`, `toasts: VecDeque<Toast>`)
3. **Action/effect layer** — `ferro-wg-tui/src/lib.rs` (mouse routing, `render_ui` ordering,
   `compute_layout` guard, `EnableMouseCapture`/`DisableMouseCapture`)

No I/O occurs in the calculation or state layers.

### New types

#### `ThemeKind` — `ferro-wg-tui-core/src/theme.rs`

```rust
/// Which Catppuccin palette variant is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeKind {
    /// Dark palette (default).
    #[default]
    Mocha,
    /// Light palette.
    Latte,
}

impl ThemeKind {
    #[must_use]
    pub fn into_theme(self) -> Theme {
        match self {
            Self::Mocha => Theme::mocha(),
            Self::Latte => Theme::latte(),
        }
    }

    #[must_use]
    pub fn toggle(self) -> Self {
        match self {
            Self::Mocha => Self::Latte,
            Self::Latte => Self::Mocha,
        }
    }
}
```

#### Official Catppuccin Mocha `Color::Rgb` values

| Semantic role | Mocha field | Hex | `Color::Rgb` |
|---|---|---|---|
| `base` | Base | `#1e1e2e` | `Color::Rgb(30, 30, 46)` |
| `surface` | Surface0 | `#313244` | `Color::Rgb(49, 50, 68)` |
| `text` | Text | `#cdd6f4` | `Color::Rgb(205, 214, 244)` |
| `subtext` | Subtext1 | `#bac2de` | `Color::Rgb(186, 194, 222)` |
| `accent` | Lavender | `#b4befe` | `Color::Rgb(180, 190, 254)` |
| `success` | Green | `#a6e3a1` | `Color::Rgb(166, 227, 161)` |
| `error` | Red | `#f38ba8` | `Color::Rgb(243, 139, 168)` |
| `warning` | Yellow | `#f9e2af` | `Color::Rgb(249, 226, 175)` |
| `muted` | Overlay0 | `#6c7086` | `Color::Rgb(108, 112, 134)` |
| `highlight_bg` | Surface1 | `#45475a` | `Color::Rgb(69, 71, 90)` |

#### Official Catppuccin Latte `Color::Rgb` values

| Semantic role | Latte field | Hex | `Color::Rgb` |
|---|---|---|---|
| `base` | Base | `#eff1f5` | `Color::Rgb(239, 241, 245)` |
| `surface` | Surface0 | `#ccd0da` | `Color::Rgb(204, 208, 218)` |
| `text` | Text | `#4c4f69` | `Color::Rgb(76, 79, 105)` |
| `subtext` | Subtext1 | `#5c5f77` | `Color::Rgb(92, 95, 119)` |
| `accent` | Lavender | `#7287fd` | `Color::Rgb(114, 135, 253)` |
| `success` | Green | `#40a02b` | `Color::Rgb(64, 160, 43)` |
| `error` | Red | `#d20f39` | `Color::Rgb(210, 15, 57)` |
| `warning` | Yellow | `#df8e1d` | `Color::Rgb(223, 142, 29)` |
| `muted` | Overlay0 | `#9ca0b0` | `Color::Rgb(156, 160, 176)` |
| `highlight_bg` | Surface1 | `#dce0e8` | `Color::Rgb(220, 224, 232)` |

#### `Toast` — `ferro-wg-tui-core/src/state.rs`

```rust
/// A single notification toast.
///
/// Replaces the single-slot `Feedback` type. Multiple toasts queue
/// simultaneously; each carries its own expiry.
#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    /// `true` for errors (rendered in `theme.error`); `false` for success.
    pub is_error: bool,
    pub expires_at: Instant,
}

/// Maximum number of toasts visible simultaneously.
pub const MAX_VISIBLE_TOASTS: usize = 5;

/// How long each toast is shown before expiring.
const TOAST_DURATION: Duration = Duration::from_secs(3);

impl Toast {
    #[must_use]
    pub fn success(message: String) -> Self {
        Self { message, is_error: false, expires_at: Instant::now() + TOAST_DURATION }
    }

    #[must_use]
    pub fn error(message: String) -> Self {
        Self { message, is_error: true, expires_at: Instant::now() + TOAST_DURATION }
    }

    #[must_use]
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}
```

`AppState` field changes:

```rust
// ferro-wg-tui-core/src/state.rs — AppState additions/replacements

pub theme_kind: ThemeKind,       // NEW  — tracks which palette is active
pub theme: Theme,                // UNCHANGED in type; value derived from theme_kind
pub toasts: VecDeque<Toast>,     // REPLACES: feedback: Option<Feedback>
pub show_help: bool,             // NEW — whether the help overlay is open
```

`AppState` helper methods:

```rust
impl AppState {
    /// Push a new toast, evicting the oldest when at capacity.
    pub fn push_toast(&mut self, toast: Toast) {
        if self.toasts.len() >= MAX_VISIBLE_TOASTS {
            self.toasts.pop_front();
        }
        self.toasts.push_back(toast);
    }

    /// Remove all expired toasts from the front of the queue.
    pub fn clear_expired_toasts(&mut self) {
        while self.toasts.front().is_some_and(Toast::is_expired) {
            self.toasts.pop_front();
        }
    }
}
```

`clear_expired_toasts` replaces `clear_expired_feedback` in the event loop. All existing
`self.feedback = Some(Feedback::success(...))` call sites become `self.push_toast(Toast::success(...))`.

### New `Action` variants — `ferro-wg-tui-core/src/action.rs`

```rust
ToggleTheme,   // switch between Catppuccin Mocha and Latte
ShowHelp,      // open the help overlay
HideHelp,      // close the help overlay
```

### New `AppEvent` variant — `ferro-wg-tui/src/event.rs`

```rust
pub enum AppEvent {
    Key(KeyEvent),
    Mouse(crossterm::event::MouseEvent),   // NEW
    Tick,
}
```

### `HelpOverlayComponent` — `ferro-wg-tui-components/src/help_overlay.rs`

Zero-state component. `handle_key`: returns `HideHelp` for `?`/`Esc`/`q` when
`state.show_help`, swallows all other keys. `render`: when `state.show_help` is `true`, renders
a `Clear`-backed overlay at 90% width × `min(height/2, 30)` rows, showing `KEYBINDINGS` from
`ferro-wg-tui-core::ux` in a two-column `Table` (50% / 50% `Constraint::Percentage`).

### `ToastComponent` — `ferro-wg-tui-components/src/toast.rs`

Zero-state, display-only component. `handle_key` returns `None` always. `render` reads
`state.toasts`, computes a rect in the bottom-right corner of the given area (width:
`min(50, area.width * 60 / 100)`, height: `state.toasts.len() as u16`), renders each toast as a
`Paragraph` line with `Clear` underneath. Success → `theme.success`; error → `theme.error`.
Newest toast at bottom.

### Keybinding table — `ferro-wg-tui-core/src/ux.rs` (new module)

```rust
/// All keybindings in display order. Single source of truth consumed by
/// `HelpOverlayComponent`.
pub const KEYBINDINGS: &[(&str, &str)] = &[
    // Global
    ("q / Esc",      "Quit"),
    ("?",            "Toggle help"),
    ("T",            "Toggle theme (Mocha/Latte)"),
    ("/",            "Search"),
    ("i",            "Import wg-quick config"),
    ("Tab / →",      "Next tab"),
    ("BackTab / ←",  "Previous tab"),
    ("1–6",          "Jump to tab"),
    ("j / ↓",        "Next row"),
    ("k / ↑",        "Previous row"),
    // Overview tab
    ("u",            "Connect all"),
    ("d",            "Disconnect all (confirm)"),
    ("s",            "Start daemon"),
    ("S",            "Stop daemon (confirm)"),
    // Status tab
    ("u",            "Connect selected"),
    ("d",            "Disconnect selected"),
    ("b",            "Cycle backend"),
    // Compare tab (Phase 5)
    ("Enter",        "Benchmark selected backend"),
    ("w",            "Switch to selected backend"),
    ("h",            "Toggle history view"),
    ("e",            "Export results"),
    // Config tab (Phase 6)
    ("e",            "Edit focused field"),
    ("p",            "Preview diff"),
    ("s",            "Save config"),
    ("r",            "Save and reconnect"),
    ("+",            "Add peer"),
    ("x",            "Delete peer (confirm)"),
    // Mouse
    ("click tab",    "Navigate to tab"),
    ("scroll ↕",     "Navigate rows"),
];
```

### Mouse routing — `ferro-wg-tui-core/src/ux.rs`

```rust
/// Convert a mouse event to an Action, given the current layout chunks.
/// `chunks[0]` is the tab bar rect produced by `compute_layout`.
#[must_use]
pub fn resolve_mouse_action(
    mouse: crossterm::event::MouseEvent,
    chunks: &[ratatui::layout::Rect],
) -> Option<Action> {
    use crossterm::event::{MouseButton, MouseEventKind};
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            tab_hit_test(mouse.column, mouse.row, chunks.first().copied()?)
        }
        MouseEventKind::ScrollDown => Some(Action::NextRow),
        MouseEventKind::ScrollUp   => Some(Action::PrevRow),
        _ => None,
    }
}
```

`tab_hit_test` accumulates cumulative widths of tab labels (format: `" N:Title "`) derived from
`Tab::ALL` and `Tab::title()` at compile time to map an x-coordinate to `Action::SelectTab(tab)`.

### Key routing chain — `ferro-wg-tui/src/lib.rs`

```rust
fn resolve_key_action(
    key: KeyEvent,
    state: &AppState,
    bundle: &ComponentBundle,
) -> Option<Action> {
    if state.show_help {
        bundle.help_overlay.handle_key(key, state)   // topmost; swallows all keys
    } else if state.config_diff_pending.is_some() {   // Phase 6 guard
        bundle.diff_preview.handle_key(key, state)
    } else if state.pending_confirm.is_some() {
        bundle.confirm_dialog.handle_key(key, state)
    } else if matches!(state.input_mode, InputMode::Search | InputMode::Import(_)
                       | InputMode::Export(_) | InputMode::EditField) {
        bundle.status_bar.handle_key(key, state)
    } else {
        handle_global_key(key)
            .or_else(|| bundle.connection_bar.handle_key(key, state))
            .or_else(|| bundle.tabs[state.active_tab.index()].handle_key(key, state))
    }
}
```

Mouse events bypass this chain entirely — they are resolved by `resolve_mouse_action` and are
**ignored** when `show_help || pending_confirm.is_some() || config_diff_pending.is_some()`.

### `render_ui` overlay ordering — `ferro-wg-tui/src/lib.rs`

```rust
terminal.draw(|frame| {
    // Minimum size guard
    if area.width < MIN_TERMINAL_WIDTH || area.height < MIN_TERMINAL_HEIGHT {
        render_too_small(frame, area, state);
        return;
    }
    // Normal render — ascending z-order
    bundle.tab_bar.render(frame, chunks[0], false, state);
    if show_bar { bundle.connection_bar.render(frame, chunks[1], false, state); }
    bundle.tabs[state.active_tab.index()].render(frame, chunks[2], true, state);
    bundle.status_bar.render(frame, chunks[3], false, state);
    bundle.toast.render(frame, chunks[2], false, state);        // above content
    bundle.confirm_dialog.render(frame, chunks[2], false, state);
    if state.config_diff_pending.is_some() {                    // Phase 6 guard
        bundle.diff_preview.render(frame, chunks[2], false, state);
    }
    bundle.help_overlay.render(frame, chunks[2], false, state); // topmost
})?;
```

### Responsive layout constants

```rust
pub const MIN_TERMINAL_WIDTH:  u16 = 80;
pub const MIN_TERMINAL_HEIGHT: u16 = 24;
```

`render_too_small` renders a single centred `Paragraph::new("Terminal too small (min 80×24)")`
with `theme.error` style and returns immediately.

---

## Implementation Steps (Commits)

### Commit 1 — Catppuccin color palette + `ThemeKind` + theme toggle

**Purpose:** Replace all placeholder `Color::*` constants with real 24-bit `Color::Rgb` values.
Add `ThemeKind`, `Action::ToggleTheme`, `Action::ShowHelp`, `Action::HideHelp`. Pure
calculation-layer change — no UI layout or component changes yet.

**Files:**
- `ferro-wg-tui-core/src/theme.rs` — add `ThemeKind` enum (`Mocha`/`Latte`) with `into_theme()`,
  `toggle()`; replace all 10 `Color::*` constants in both `mocha()` and `latte()` with the
  `Color::Rgb` values from the tables above; update existing color tests to use the new RGB values
- `ferro-wg-tui-core/src/action.rs` — add `ToggleTheme`, `ShowHelp`, `HideHelp`
- `ferro-wg-tui-core/src/state.rs` — add `theme_kind: ThemeKind` and `show_help: bool` to
  `AppState`; update `AppState::new()` to derive `theme` from `ThemeKind::default().into_theme()`;
  add dispatch arms: `ToggleTheme` → toggle `theme_kind` and rebuild `theme`; `ShowHelp` /
  `HideHelp` → set `show_help`
- `ferro-wg-tui-core/src/lib.rs` — re-export `ThemeKind`
- `ferro-wg-tui/src/lib.rs` — add `Char('T') => Some(Action::ToggleTheme)` and
  `Char('?') => Some(Action::ShowHelp)` to `handle_global_key`
- `ferro-wg-tui-components/Cargo.toml` — add `insta = { version = "1", features = ["yaml"] }` to
  `[dev-dependencies]`
- `ferro-wg-tui-core/Cargo.toml` — same

**Tests:**
- `ThemeKind::Mocha.toggle()` → `Latte`; `Latte.toggle()` → `Mocha`
- `ThemeKind::Mocha.into_theme().accent` → `Color::Rgb(180, 190, 254)`
- `ThemeKind::Latte.into_theme().accent` → `Color::Rgb(114, 135, 253)`
- `Theme::mocha().base` → `Color::Rgb(30, 30, 46)`
- `Theme::latte().base` → `Color::Rgb(239, 241, 245)`
- `AppState::dispatch(ToggleTheme)` from `Mocha` → `theme_kind == Latte`
- `AppState::dispatch(ToggleTheme)` × 2 → back to `Mocha`
- `AppState::dispatch(ShowHelp)` → `show_help == true`
- `AppState::dispatch(HideHelp)` → `show_help == false`
- Snapshot (TestBackend 80×24, Mocha): `TabBarComponent` renders with accent `Color::Rgb(180, 190, 254)`
- Snapshot (TestBackend 80×24, Latte): accent → `Color::Rgb(114, 135, 253)`

---

### Commit 2 — Toast queue (replaces single-slot `Feedback`)

**Purpose:** Replace `AppState::feedback: Option<Feedback>` with `AppState::toasts:
VecDeque<Toast>`. All existing call sites updated. New `ToastComponent` renders the queue.

**Files:**
- `ferro-wg-tui-core/src/state.rs` — add `Toast` struct with `success`, `error`, `is_expired`;
  add `MAX_VISIBLE_TOASTS`, `TOAST_DURATION` constants; add `push_toast`, `clear_expired_toasts`
  to `AppState`; rename `feedback: Option<Feedback>` → `toasts: VecDeque<Toast>`; update
  `dispatch(DaemonOk)` and `dispatch(DaemonError)` to call `push_toast`
- `ferro-wg-tui-core/src/lib.rs` — re-export `Toast`, `MAX_VISIBLE_TOASTS`; remove or alias
  `Feedback`
- `ferro-wg-tui-components/src/toast.rs` — new file: `ToastComponent` implementing `Component`;
  `handle_key` always `None`; `render` places toasts in bottom-right corner with `Clear` underlay
- `ferro-wg-tui-components/src/lib.rs` — add `pub mod toast; pub use toast::ToastComponent`
- `ferro-wg-tui/src/lib.rs` — add `toast: ToastComponent` to `ComponentBundle`; add toast render
  call after active tab content; replace `clear_expired_feedback()` with `clear_expired_toasts()`
- `ferro-wg-tui-components/src/status_bar.rs` — remove `state.feedback` branch from `render`;
  status bar reverts to showing only `InputMode` content and daemon indicator

**Tests:**
- `Toast::success("ok")` — `is_error == false`, not expired immediately
- `Toast::error("fail")` — `is_error == true`
- `push_toast` with 5 existing → oldest evicted, `toasts.len() == 5`
- `clear_expired_toasts` — only expired front entries removed
- `dispatch(DaemonOk("hello"))` → `toasts.back().message == "hello"`
- `dispatch(DaemonError("fail"))` → `toasts.back().is_error == true`
- `dispatch(DaemonOk)` twice → `toasts.len() == 2`
- Snapshot (TestBackend 80×24, 2 toasts): overlay in bottom-right; contains both messages

---

### Commit 3 — Help overlay component

**Purpose:** Add `HelpOverlayComponent` and `KEYBINDINGS` table. Wire into `ComponentBundle` and
`render_ui`. After this commit `?` opens the overlay and `Esc`/`q`/`?` close it.

**Files:**
- `ferro-wg-tui-core/src/ux.rs` — new module: `pub const KEYBINDINGS: &[(&str, &str)]` (full
  table from architecture section)
- `ferro-wg-tui-core/src/lib.rs` — add `pub mod ux; pub use ux::KEYBINDINGS`
- `ferro-wg-tui-components/src/util.rs` — extract `centered_rect` here from `confirm_dialog.rs`
  if Phase 6 has not already done so
- `ferro-wg-tui-components/src/confirm_dialog.rs` — replace private `centered_rect` with
  `use crate::util::centered_rect` if extracting now
- `ferro-wg-tui-components/src/help_overlay.rs` — new file: `HelpOverlayComponent`; `handle_key`
  returns `HideHelp` for `?`/`Esc`/`q` when overlay is open, swallows all other keys; `render`
  shows `KEYBINDINGS` in a two-column `Table` within a `Clear`-backed overlay at 90% width
- `ferro-wg-tui-components/src/lib.rs` — add `pub mod help_overlay; pub use help_overlay::HelpOverlayComponent`
- `ferro-wg-tui/src/lib.rs` — add `help_overlay: HelpOverlayComponent` to `ComponentBundle`;
  add `state.show_help` guard at top of `resolve_key_action`; add topmost render call; update
  `dispatch_all`

**Tests:**
- `handle_key(?, show_help=true)` → `Some(HideHelp)`
- `handle_key(Esc, show_help=true)` → `Some(HideHelp)`
- `handle_key(q, show_help=true)` → `Some(HideHelp)`
- `handle_key(j, show_help=true)` → `None` (swallowed, no row navigation leaks through)
- `KEYBINDINGS.len()` ≥ 10
- `KEYBINDINGS` contains an entry whose key label is `"?"`
- Snapshot (80×24, `show_help=false`): no overlay
- Snapshot (80×24, `show_help=true`): overlay with "Help" title; contains `"q / Esc"` and `"Toggle theme"`
- Snapshot (120×40, `show_help=true`): overlay wider; two columns visible

---

### Commit 4 — Mouse support

**Purpose:** Enable `MouseCapture`, add `AppEvent::Mouse`, route mouse events through
`resolve_mouse_action`, implement tab-click and scroll.

**Files:**
- `ferro-wg-tui/src/event.rs` — add `AppEvent::Mouse(crossterm::event::MouseEvent)`; forward
  `Event::Mouse(m)` in the event-loop `maybe_event` arm
- `ferro-wg-tui-core/src/ux.rs` — implement `resolve_mouse_action` and `tab_hit_test` /
  `tab_label_at_column`; export `resolve_mouse_action`
- `ferro-wg-tui/src/lib.rs` — enable `EnableMouseCapture` in `run()` after `EnterAlternateScreen`;
  add `DisableMouseCapture` to the cleanup block; add `AppEvent::Mouse(m)` arm to the event loop
  that calls `handle_input_event` with mouse; ignore mouse when
  `show_help || pending_confirm.is_some() || config_diff_pending.is_some()`
- `ferro-wg-tui-core/src/component.rs` — add default method
  `fn handle_mouse(&mut self, _mouse: crossterm::event::MouseEvent, _state: &AppState) -> Option<Action> { None }`

**Tests:**
- `resolve_mouse_action(ScrollDown, chunks)` → `Some(NextRow)`
- `resolve_mouse_action(ScrollUp, chunks)` → `Some(PrevRow)`
- `resolve_mouse_action(right-click, chunks)` → `None`
- `resolve_mouse_action(mouse-move, chunks)` → `None`
- `tab_hit_test(col_in_first_label, row_in_tab_bar, rect)` → `Some(SelectTab(Overview))`
- `tab_hit_test(col, row_below_tab_bar, rect)` → `None`
- `tab_label_at_column(0, origin_x=0)` → `Some(Tab::Overview)`
- `tab_label_at_column(200, origin_x=0)` → `None` (past all labels)
- Guard test: with `state.show_help = true`, verify mouse scroll does not advance
  `selected_peer_row` (guard returns before dispatch)

---

### Commit 5 — Responsive layout (80×24 minimum)

**Purpose:** Enforce the 80×24 minimum, add the `too-small` fallback render, and verify all
component renders are panic-free at minimum size.

**Files:**
- `ferro-wg-tui/src/lib.rs` — add `MIN_TERMINAL_WIDTH: u16 = 80` and
  `MIN_TERMINAL_HEIGHT: u16 = 24`; add `render_too_small`; add size guard at top of `render_ui`
  closure
- All tab components in `ferro-wg-tui-components/src/` — add early return guard at top of
  `render`: `if area.height == 0 || area.width < 20 { return; }`
- `ferro-wg-tui-components/src/status.rs` — verify the two-pane `Layout::split` uses
  `Constraint::Min(0)` for the peer table so it collapses without panic when height is minimal

**Tests (all `insta` snapshot-based using `TestBackend`):**
- `OverviewComponent`, 80×24, empty connections → `"No connections"` placeholder
- `OverviewComponent`, 80×24, 1 connection → row renders without truncation panic
- `OverviewComponent`, 120×40, 1 connection → wider layout
- `StatusComponent`, 80×24 → no panic
- `PeersComponent`, 80×24 → no panic
- `CompareComponent`, 80×24 → placeholder dashes visible
- `ConfigComponent`, 80×24 → field labels visible
- `LogsComponent`, 80×24 → `"No log entries"` or empty
- Full `render_ui` at 79×24 → `"Terminal too small"` message visible
- Full `render_ui` at 80×23 → `"Terminal too small"` message visible
- Full `render_ui` at 80×24 → normal render (all chrome bars present)
- Unit: `compute_layout(80×24)` → 4 chunks; `chunks[0].height == TAB_BAR_HEIGHT`;
  `chunks[3].height == STATUS_BAR_HEIGHT`; `chunks[2].height ≥ 1`

---

## Dependency Graph

```
Commit 1 (ThemeKind + Action variants)
  └─> Commit 2 (Toast queue) — shares Action enum from Commit 1
  └─> Commit 3 (Help overlay) — reads theme.overlay_block; uses ShowHelp/HideHelp from Commit 1
  └─> Commit 4 (Mouse) — uses Action::NextRow/PrevRow; needs AppEvent::Mouse (independent of theme)
  └─> Commit 5 (Responsive) — reads theme.error for too-small message; otherwise independent
```

Each commit leaves the codebase in a compilable, tested state. Commits 2–5 can be developed in
parallel after Commit 1 lands.

---

## Testing Strategy

### Framework

Add to `[dev-dependencies]` in `ferro-wg-tui-components/Cargo.toml` and
`ferro-wg-tui-core/Cargo.toml`:

```toml
insta = { version = "1", features = ["yaml"] }
```

Snapshot tests use `ratatui::backend::TestBackend` to render into a virtual buffer, then call
`insta::assert_snapshot!(terminal_to_string(&terminal))`.

### `terminal_to_string` helper

```rust
/// Collect the terminal buffer into a human-readable grid for snapshot diffing.
fn terminal_to_string(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer().clone();
    let width = buffer.area().width as usize;
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(ratatui::buffer::Cell::symbol).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
```

Place in `ferro-wg-tui-components/tests/common.rs` (shared across snapshot test modules).

### Snapshot naming convention

`<component>__<size>__<variant>` — examples:

```
tab_bar__80x24__mocha
tab_bar__80x24__latte
overview__80x24__empty
overview__80x24__one_connection
help_overlay__80x24__open
help_overlay__120x40__open
toast__80x24__two_toasts
full_render__79x24__too_small
full_render__80x24__normal
```

Run `cargo insta review` after the first test run to approve initial snapshots. Snapshots live in
`ferro-wg-tui-components/src/snapshots/`.

### Unit test summary

| Module | Test | Assertion |
|---|---|---|
| `theme.rs` | `mocha_uses_catppuccin_colors` | `mocha().accent == Color::Rgb(180, 190, 254)` |
| `theme.rs` | `latte_uses_catppuccin_colors` | `latte().base == Color::Rgb(239, 241, 245)` |
| `theme.rs` | `theme_kind_toggle` | `Mocha.toggle() == Latte` and vice-versa |
| `state.rs` | `toggle_theme_action` | `dispatch(ToggleTheme)` twice returns to original |
| `state.rs` | `push_toast_evicts_oldest` | queue at cap + one push → oldest evicted |
| `state.rs` | `clear_expired_toasts_only_removes_front` | expired front removed; live front kept |
| `ux.rs` | `mouse_scroll_down` | `resolve_mouse_action(ScrollDown, _) == Some(NextRow)` |
| `ux.rs` | `mouse_scroll_up` | `resolve_mouse_action(ScrollUp, _) == Some(PrevRow)` |
| `ux.rs` | `tab_hit_test_first_tab` | click at col 1 inside tab bar → `SelectTab(Overview)` |
| `ux.rs` | `tab_hit_test_outside` | click below tab bar → `None` |
| `help_overlay.rs` | `esc_closes_overlay` | `handle_key(Esc, show_help=true)` → `HideHelp` |
| `help_overlay.rs` | `j_swallowed_when_open` | `handle_key(j, show_help=true)` → `None` |
| `lib.rs` (tui) | `compute_layout_80x24` | 4 chunks; content height ≥ 1 |
| `lib.rs` (tui) | `render_too_small_at_79x24` | buffer contains "too small" text |

---

## Keybinding conflict analysis

| Key | Existing binding | Phase 7 binding | Conflict? |
|-----|----------------|-----------------|-----------|
| `?` | none | `ShowHelp` (global) | No — previously unmapped |
| `T` (uppercase) | none | `ToggleTheme` (global) | No — previously unmapped |
| `Esc` in help overlay | `Quit` (normal mode) | `HideHelp` (help overlay mode) | No — mode-gated |
| `q` in help overlay | `Quit` (normal mode) | `HideHelp` (help overlay mode) | No — mode-gated |
| mouse scroll | ignored (`Event::Mouse` discarded) | `NextRow` / `PrevRow` | No — previously discarded |
| mouse click tab bar | ignored | `SelectTab(n)` | No — previously discarded |

---

## Out-of-scope (MVP boundary)

- Custom user themes beyond Catppuccin Mocha and Latte
- Configurable keybindings (`KEYBINDINGS` is the foundation; the editing UI is deferred)
- Session restore (last tab, scroll position)
- Sixel/Kitty image protocol for richer charts in the Compare tab
- Mouse drag (selecting text, resizing panels)
- Mouse click to select a row (only scroll implemented; row-click requires per-component y→row
  mapping — deferred)
- Accessibility features (screen reader support, high-contrast mode)

---

## Tooling checklist (per commit)

```
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings -D clippy::pedantic
cargo test --workspace --features boringtun,neptun,gotatun
cargo insta review   # after first snapshot test run to approve initial snapshots
```
