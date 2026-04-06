//! User experience constants shared across the TUI.
//!
//! Mouse-event resolution and tab hit-testing live in
//! `ferro_wg_tui_components::tab_bar`, where the tab geometry is defined.

/// Full table of keybindings for the help overlay.
///
/// Organised by sections: Global, Overview, Status, Compare, Config, Mouse.
/// Each entry is `(key_label, description)`.
pub const KEYBINDINGS: &[(&str, &str)] = &[
    // Global
    ("q / Esc", "Quit"),
    ("?", "Toggle help"),
    ("T", "Toggle theme (Mocha/Latte)"),
    ("/", "Search"),
    ("i", "Import wg-quick config"),
    ("Tab / →", "Next tab"),
    ("BackTab / ←", "Previous tab"),
    ("1–6", "Jump to tab"),
    ("j / ↓", "Next row"),
    ("k / ↑", "Previous row"),
    // Overview tab
    ("u", "Connect all"),
    ("d", "Disconnect all (confirm)"),
    ("s", "Start daemon"),
    ("S", "Stop daemon (confirm)"),
    // Status tab
    ("u", "Connect selected"),
    ("d", "Disconnect selected"),
    ("b", "Cycle backend"),
    // Compare tab (Phase 5)
    ("Enter", "Benchmark selected backend"),
    ("w", "Switch to selected backend"),
    ("h", "Toggle history view"),
    ("e", "Export results"),
    // Config tab (Phase 6)
    ("e", "Edit focused field"),
    ("p", "Preview diff"),
    ("s", "Save config"),
    ("r", "Save and reconnect"),
    ("+", "Add peer"),
    ("x", "Delete peer (confirm)"),
    // Mouse
    ("click tab", "Navigate to tab"),
    ("scroll ↕", "Navigate rows"),
];
