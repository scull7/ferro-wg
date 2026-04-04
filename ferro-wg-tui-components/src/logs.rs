#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn test_parse_log_line_with_timestamp_and_level() {
        let line = "12:34:56 INFO ferro_wg_core::tunnel::mod: Connection abc is up";
        let spans = LogsComponent::parse_log_line(line);

        assert_eq!(spans.len(), 5);
        assert_eq!(spans[0].content, "[12:34:56]");
        assert_eq!(spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(spans[2].content, "[INFO]");
        assert_eq!(spans[2].style.fg, Some(Color::Green));
        assert_eq!(
            spans[4].content,
            "ferro_wg_core::tunnel::mod: Connection abc is up"
        );
    }

    #[test]
    fn test_parse_log_line_error_level() {
        let line = "12:34:56 ERROR ferro_wg_core::error: Failed to connect";
        let spans = LogsComponent::parse_log_line(line);

        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content, "[ERROR]");
        assert_eq!(spans[2].style.fg, Some(Color::Red));
    }

    #[test]
    fn test_parse_log_line_warn_level() {
        let line = "12:34:56 WARN ferro_wg_core::tunnel: Handshake timeout";
        let spans = LogsComponent::parse_log_line(line);

        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content, "[WARN]");
        assert_eq!(spans[2].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn test_parse_log_line_debug_level() {
        let line = "12:34:56 DEBUG ferro_wg_core::stats: Packet count: 42";
        let spans = LogsComponent::parse_log_line(line);

        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content, "[DEBUG]");
        assert_eq!(spans[2].style.fg, Some(Color::Blue));
    }

    #[test]
    fn test_parse_log_line_legacy_format() {
        let line = "INFO ferro_wg_core::tunnel::mod: Connection abc is up";
        let spans = LogsComponent::parse_log_line(line);

        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, line);
    }

    #[test]
    fn test_parse_log_line_malformed() {
        let line = "some random log message";
        let spans = LogsComponent::parse_log_line(line);

        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, line);
    }
}
