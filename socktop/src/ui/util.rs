//! Small UI helpers: human-readable sizes, truncation, icons.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Gauge},
};

/// Bordered gauge with a centered label that stays readable over the bar.
///
/// ratatui draws the part of the label that sits on the filled bar in the
/// terminal's default foreground on the bar colour — light text on a bright
/// bar in a dark theme. Those cells get black text instead; the part of the
/// label past the bar keeps its normal colour.
pub fn draw_labeled_gauge(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    title: &str,
    fill: Color,
    percent: u16,
    label: String,
) {
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(title))
        .gauge_style(Style::default().fg(fill))
        .percent(percent.min(100))
        .label(label);
    f.render_widget(gauge, area);

    // Label cells over the bar are the only ones painted with the fill as
    // their *background* (the bar itself is foreground block glyphs).
    let buf = f.buffer_mut();
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            if cell.bg == fill {
                cell.set_fg(Color::Black);
            }
        }
    }
}

pub fn human(b: u64) -> String {
    const K: f64 = 1024.0;
    let b = b as f64;
    if b < K {
        return format!("{b:.0}B");
    }
    let kb = b / K;
    if kb < K {
        return format!("{kb:.1}KB");
    }
    let mb = kb / K;
    if mb < K {
        return format!("{mb:.1}MB");
    }
    let gb = mb / K;
    if gb < K {
        return format!("{gb:.1}GB");
    }
    let tb = gb / K;
    format!("{tb:.2}TB")
}

pub fn disk_icon(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    if n.contains(':') {
        "🗄️"
    } else if n.contains("nvme") {
        "⚡"
    } else if n.starts_with("sd") {
        "💽"
    } else if n.contains("overlay") {
        "📦"
    } else {
        "🖴"
    }
}

#[cfg(test)]
mod gauge_label_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Foreground colour of every label character, left to right.
    fn label_colors(percent: u16) -> Vec<(String, Color, Color)> {
        let mut terminal = Terminal::new(TestBackend::new(40, 3)).unwrap();
        terminal
            .draw(|f| {
                draw_labeled_gauge(
                    f,
                    Rect::new(0, 0, 40, 3),
                    "Memory",
                    Color::Magenta,
                    percent,
                    "39.2GB / 48.0GB".into(),
                )
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // Inner row is y = 1; the 15-char label is centered in 38 columns.
        (12..27)
            .map(|x| {
                let c = &buf[(x, 1)];
                (c.symbol().to_string(), c.fg, c.bg)
            })
            .collect()
    }

    #[test]
    fn label_is_black_only_where_the_bar_covers_it() {
        // 50% of 38 columns fills through x = 19: "39.2GB /" is covered.
        let cells = label_colors(50);
        let text: String = cells.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(text, "39.2GB / 48.0GB");
        for (i, (sym, fg, bg)) in cells.iter().enumerate() {
            if i < 8 {
                assert_eq!((*fg, *bg), (Color::Black, Color::Magenta), "{i} {sym:?}");
            } else {
                assert_ne!(*fg, Color::Black, "{i} {sym:?}");
                assert_ne!(*bg, Color::Magenta, "{i} {sym:?}");
            }
        }
    }

    #[test]
    fn label_is_all_black_when_full_and_untouched_when_empty() {
        assert!(label_colors(100).iter().all(|c| c.1 == Color::Black));
        assert!(label_colors(0).iter().all(|c| c.1 != Color::Black));
    }
}
