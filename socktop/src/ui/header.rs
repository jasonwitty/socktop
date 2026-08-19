//! Top header with hostname, connection status and polling intervals.
//!
//! The row carries two pieces of text — session identity on the left, polling intervals
//! on the right — and both matter. Rather than let the right one overwrite the left when
//! they no longer both fit, the header drops detail in priority order: the hostname and
//! the intervals are what survive longest, because they are what tells you *which* host
//! you are looking at and how fresh the numbers are.

use crate::ui::fit::{cols, pick_pair, truncate_cols};
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

/// Columns kept clear between the left and right halves.
const GAP: u16 = 2;
/// Never shorten the hostname below this before dropping the intervals instead.
const HOSTNAME_FLOOR: u16 = 8;

/// Session state the header renders.
#[derive(Clone, Copy)]
pub struct HeaderState<'a> {
    pub hostname: Option<&'a str>,
    pub is_tls: bool,
    pub has_token: bool,
    pub metrics_ms: u128,
    pub procs_ms: u128,
}

/// Builds the left and right halves of the header for a row `width` columns wide.
///
/// Detail is dropped in this order as the row narrows: the key hints, then the TLS/token
/// badges, then the `socktop — host:` prefix (leaving the bare hostname), then the
/// `metrics`/`procs` words, and only then is the hostname itself shortened. The two
/// halves are always sized to sit side by side, so neither can paint over the other.
///
/// Callers cache the result and rebuild it only when the state or the width changes.
pub fn build_header(state: HeaderState<'_>, width: u16) -> (String, String) {
    let host = state.hostname.unwrap_or("connecting...");
    let tls = if state.is_tls {
        "🔒 TLS"
    } else {
        "🔒✗ TLS"
    };
    let badges = if state.has_token {
        format!("{tls} | 🔑 token")
    } else {
        tls.to_string()
    };

    let named = format!("socktop — host: {host}");
    let with_badges = format!("{named} | {badges}");
    let with_keys = format!("{with_badges} | (a: about, h: help, q: quit)");

    let intervals = format!(
        "⏱ {}ms metrics | {}ms procs",
        state.metrics_ms, state.procs_ms
    );
    let intervals_short = format!("⏱ {}ms | {}ms", state.metrics_ms, state.procs_ms);

    // Richest first. The bare hostname is reached before the intervals lose their
    // labels, and the hostname is only shortened once nothing else is left to give.
    let ladder = [
        (with_keys.as_str(), intervals.as_str()),
        (with_badges.as_str(), intervals.as_str()),
        (named.as_str(), intervals.as_str()),
        (host, intervals.as_str()),
        (host, intervals_short.as_str()),
    ];
    let (left, right) = pick_pair(width, GAP, &ladder);
    if cols(left) + cols(right) + GAP <= width {
        return (left.to_string(), right.to_string());
    }

    // Past the floor of the ladder: shorten the hostname, and give up the intervals only
    // if even a stub of a hostname will not fit beside them.
    let room = width
        .saturating_sub(cols(&intervals_short))
        .saturating_sub(GAP);
    if room >= HOSTNAME_FLOOR {
        return (truncate_cols(host, room), intervals_short);
    }
    (truncate_cols(host, width), String::new())
}

pub fn draw_header(f: &mut ratatui::Frame<'_>, area: Rect, title: &str, intervals: &str) {
    f.render_widget(Block::default().title(title).borders(Borders::BOTTOM), area);

    if intervals.is_empty() {
        return;
    }
    let intervals_width = cols(intervals);
    if area.width >= intervals_width {
        let right_area = Rect {
            x: area.x + area.width - intervals_width,
            y: area.y,
            width: intervals_width,
            height: 1,
        };
        f.render_widget(Paragraph::new(Line::from(Span::raw(intervals))), right_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(hostname: Option<&str>) -> HeaderState<'_> {
        HeaderState {
            hostname,
            is_tls: false,
            has_token: false,
            metrics_ms: 500,
            procs_ms: 2000,
        }
    }

    /// The defect this replaces: the two halves were painted independently, so below
    /// ~105 columns the right half landed on top of the title. Whatever the width, they
    /// must now fit side by side.
    #[test]
    fn halves_never_overlap_at_any_width() {
        for width in 0..=200u16 {
            let (left, right) = build_header(state(Some("cachyos-gaming")), width);
            let used = cols(&left) + cols(&right);
            if right.is_empty() {
                assert!(cols(&left) <= width, "width {width}: {left:?} overflows");
            } else {
                assert!(
                    used + GAP <= width,
                    "width {width}: {left:?} + {right:?} = {used} cols, no room for both"
                );
            }
        }
    }

    /// Hostname and intervals are the two things worth keeping; everything else is
    /// context that can go.
    #[test]
    fn hostname_and_intervals_survive_longest() {
        for width in 34..=200u16 {
            let (left, right) = build_header(state(Some("cachyos-gaming")), width);
            assert!(
                left.contains("cachyos-gaming"),
                "width {width}: lost the hostname ({left:?})"
            );
            assert!(
                right.contains("500ms") && right.contains("2000ms"),
                "width {width}: lost the intervals ({right:?})"
            );
        }
    }

    /// The ladder from the design: key hints, then badges, then the prefix, then the
    /// interval labels, then the hostname itself.
    #[test]
    fn detail_is_dropped_in_priority_order() {
        let s = state(Some("cachyos-gaming"));

        let (left, right) = build_header(s, 120);
        assert_eq!(
            left,
            "socktop — host: cachyos-gaming | 🔒✗ TLS | (a: about, h: help, q: quit)"
        );
        assert_eq!(right, "⏱ 500ms metrics | 2000ms procs");

        // Key hints go first.
        let (left, _) = build_header(s, 80);
        assert_eq!(left, "socktop — host: cachyos-gaming | 🔒✗ TLS");

        // Then the badges.
        let (left, _) = build_header(s, 70);
        assert_eq!(left, "socktop — host: cachyos-gaming");

        // Then the prefix, leaving the bare hostname.
        let (left, right) = build_header(s, 50);
        assert_eq!(left, "cachyos-gaming");
        assert_eq!(right, "⏱ 500ms metrics | 2000ms procs");

        // Then the interval labels.
        let (left, right) = build_header(s, 34);
        assert_eq!(left, "cachyos-gaming");
        assert_eq!(right, "⏱ 500ms | 2000ms");

        // Only then is the hostname itself shortened.
        // 30 columns - 16 for the short intervals - 2 gap leaves 12 for the hostname.
        let (left, right) = build_header(s, 30);
        assert_eq!(left, "cachyos-gam…");
        assert_eq!(right, "⏱ 500ms | 2000ms");
    }

    /// A long hostname must not push the intervals off the row.
    #[test]
    fn a_long_hostname_is_shortened_rather_than_winning_the_row() {
        let long = "a-very-long-hostname-that-will-not-fit-anywhere";
        for width in 30..=100u16 {
            let (left, right) = build_header(state(Some(long)), width);
            assert!(!right.is_empty(), "width {width}: intervals were dropped");
            assert!(cols(&left) + cols(&right) + GAP <= width, "width {width}");
        }
    }

    /// Widths too small for both: the hostname is the last thing standing.
    #[test]
    fn hostname_is_the_final_survivor() {
        let (left, right) = build_header(state(Some("cachyos-gaming")), 20);
        assert!(right.is_empty(), "intervals should have been dropped");
        assert!(!left.is_empty());
        assert!(cols(&left) <= 20);
    }

    #[test]
    fn tls_and_token_badges_appear_when_there_is_room() {
        let s = HeaderState {
            hostname: Some("host"),
            is_tls: true,
            has_token: true,
            metrics_ms: 500,
            procs_ms: 2000,
        };
        let (left, _) = build_header(s, 200);
        assert!(left.contains("🔒 TLS"), "{left}");
        assert!(left.contains("🔑 token"), "{left}");
    }

    #[test]
    fn a_missing_hostname_reads_as_connecting() {
        let (left, _) = build_header(state(None), 120);
        assert!(left.contains("connecting"), "{left}");
    }
}
