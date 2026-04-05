//! Shared utilities for TUI components.

/// Compute a centered [`Rect`] with a percentage width and fixed height.
///
/// `pct_x` is the desired width as a percentage of `area.width` (0–100).
/// The returned rect is clamped to fit within `area`.
pub(crate) fn centered_rect(
    pct_x: u16,
    height: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    let width = area.width * pct_x / 100;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    ratatui::layout::Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn area(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    #[test]
    fn centered_rect_normal_case() {
        let r = centered_rect(60, 5, area(80, 20));
        assert_eq!(r.width, 48); // 80 * 60 / 100
        assert_eq!(r.height, 5);
        assert_eq!(r.x, 16); // (80 - 48) / 2
        assert_eq!(r.y, 7); // (20 - 5) / 2
    }

    #[test]
    fn centered_rect_full_width() {
        let r = centered_rect(100, 3, area(80, 20));
        assert_eq!(r.width, 80);
        assert_eq!(r.x, 0);
    }

    #[test]
    fn centered_rect_zero_percent_width() {
        let r = centered_rect(0, 3, area(80, 20));
        assert_eq!(r.width, 0);
    }

    #[test]
    fn centered_rect_height_clamped_to_area() {
        let r = centered_rect(60, 30, area(80, 10));
        assert_eq!(r.height, 10);
        assert_eq!(r.y, 0);
    }

    #[test]
    fn centered_rect_zero_width_area_no_panic() {
        let r = centered_rect(60, 3, area(0, 10));
        assert_eq!(r.width, 0);
    }
}
