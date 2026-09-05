//! Three-row terminal digits. Unsupported or oversized values use caller fallbacks.
use ratatui::{
    layout::{Alignment, Rect},
    style::Style,
    text::Line,
    widgets::Paragraph,
    Frame,
};

fn glyph(ch: char) -> Option<[&'static str; 3]> {
    Some(match ch {
        '0' => ["█▀█", "█ █", "█▄█"],
        '1' => [" ▀█", "  █", "  █"],
        '2' => ["▀▀█", "█▀▀", "█▄▄"],
        '3' => ["▀▀█", " ▀█", "▄▄█"],
        '4' => ["█ █", "▀▀█", "  █"],
        '5' => ["█▀▀", "▀▀█", "▄▄█"],
        '6' => ["█▀▀", "█▀█", "█▄█"],
        '7' => ["▀▀█", "  █", "  █"],
        '8' => ["█▀█", "█▀█", "█▄█"],
        '9' => ["█▀█", "▀▀█", "▄▄█"],
        '.' => [" ", " ", "▄"],
        '-' | '—' => ["   ", "▀▀▀", "   "],
        _ => return None,
    })
}

/// Draw only when the complete value fits. Never truncate or round a measurement.
pub(super) fn draw(
    frame: &mut Frame,
    area: Rect,
    value: &str,
    style: Style,
    alignment: Alignment,
) -> bool {
    if area.height < 3 || value.is_empty() || value.len() > 64 {
        return false;
    }
    let Some(glyphs) = value.chars().map(glyph).collect::<Option<Vec<_>>>() else {
        return false;
    };
    let width =
        glyphs.iter().map(|g| g[0].chars().count()).sum::<usize>() + glyphs.len().saturating_sub(1);
    if width > usize::from(area.width) {
        return false;
    }
    let lines = (0..3)
        .map(|row| Line::from(glyphs.iter().map(|g| g[row]).collect::<Vec<_>>().join(" ")))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).style(style).alignment(alignment),
        area,
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn rejects_unsupported_or_oversize_without_drawing_partial_values() {
        for value in ["NaN", "1e10", "1000000000.0", ""] {
            let mut terminal = Terminal::new(TestBackend::new(12, 3)).unwrap();
            terminal
                .draw(|frame| {
                    assert!(!draw(
                        frame,
                        frame.area(),
                        value,
                        Style::default(),
                        Alignment::Left
                    ));
                })
                .unwrap();
            assert!(terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|cell| cell.symbol() == " "));
        }
    }

    #[test]
    fn digit_rows_have_consistent_cell_widths() {
        for ch in "0123456789.-—".chars() {
            let rows = glyph(ch).unwrap();
            assert_eq!(rows[0].chars().count(), rows[1].chars().count());
            assert_eq!(rows[0].chars().count(), rows[2].chars().count());
        }
    }
}
