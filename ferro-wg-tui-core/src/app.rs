//! Tab and input-mode enums shared across TUI crates.

/// Active tab in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// Active tunnel overview.
    Status,
    /// All configured peers.
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
    pub const ALL: [Self; 5] = [
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
            Self::Status => 0,
            Self::Peers => 1,
            Self::Compare => 2,
            Self::Config => 3,
            Self::Logs => 4,
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

/// Input mode — normal navigation or search filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// Arrow keys navigate, hotkeys active.
    Normal,
    /// Typing into the search bar.
    Search,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_titles() {
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
        assert_eq!(Tab::Status.next(), Tab::Peers);
        assert_eq!(Tab::Logs.next(), Tab::Status);
    }

    #[test]
    fn tab_prev_wraps() {
        assert_eq!(Tab::Status.prev(), Tab::Logs);
        assert_eq!(Tab::Peers.prev(), Tab::Status);
    }
}
