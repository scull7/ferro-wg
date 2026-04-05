//! Tab and input-mode enums shared across TUI crates.

/// Active tab in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// Aggregate health overview for all connections.
    Overview,
    /// Active tunnel overview (scoped to selected connection).
    Status,
    /// All configured peers (scoped to selected connection).
    Peers,
    /// Backend performance comparison.
    Compare,
    /// Interface and peer configuration.
    Config,
    /// Live log viewer.
    Logs,
}

impl Tab {
    /// All tabs in display order.
    pub const ALL: [Self; 6] = [
        Self::Overview,
        Self::Status,
        Self::Peers,
        Self::Compare,
        Self::Config,
        Self::Logs,
    ];

    /// Tab display title.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Status => "Status",
            Self::Peers => "Peers",
            Self::Compare => "Compare",
            Self::Config => "Config",
            Self::Logs => "Logs",
        }
    }

    /// Zero-based tab index.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Self::Overview => 0,
            Self::Status => 1,
            Self::Peers => 2,
            Self::Compare => 3,
            Self::Config => 4,
            Self::Logs => 5,
        }
    }

    /// Next tab, wrapping around.
    #[must_use]
    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    /// Previous tab, wrapping around.
    #[must_use]
    pub fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// Input mode — normal navigation, search filtering, or import path entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    /// Arrow keys navigate, hotkeys active.
    Normal,
    /// Typing into the search bar.
    Search,
    /// Typing an import file path. Inner `String` is the current buffer.
    Import(String),
    /// Typing an export file path. Inner `String` is the current buffer.
    Export(String),
    /// Editing a single config field. Buffer lives in `AppState::config_edit`.
    EditField,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_titles() {
        assert_eq!(Tab::Overview.title(), "Overview");
        assert_eq!(Tab::Status.title(), "Status");
        assert_eq!(Tab::Peers.title(), "Peers");
        assert_eq!(Tab::Compare.title(), "Compare");
        assert_eq!(Tab::Config.title(), "Config");
        assert_eq!(Tab::Logs.title(), "Logs");
    }

    #[test]
    fn tab_indices() {
        for (i, tab) in Tab::ALL.iter().enumerate() {
            assert_eq!(tab.index(), i);
        }
    }

    #[test]
    fn tab_next_wraps() {
        assert_eq!(Tab::Overview.next(), Tab::Status);
        assert_eq!(Tab::Status.next(), Tab::Peers);
        assert_eq!(Tab::Logs.next(), Tab::Overview);
    }

    #[test]
    fn tab_prev_wraps() {
        assert_eq!(Tab::Overview.prev(), Tab::Logs);
        assert_eq!(Tab::Status.prev(), Tab::Overview);
        assert_eq!(Tab::Peers.prev(), Tab::Status);
    }
}
