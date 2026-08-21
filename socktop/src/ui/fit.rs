//! Fitting text to the columns actually available.
//!
//! Several panes paint two independent pieces of text onto one row — a left title and a
//! right-aligned readout. Nothing reserves space for the right piece, so on a narrow
//! terminal the right one is simply painted over the tail of the left one and the title
//! is clobbered mid-word. The helpers here let a caller measure in real terminal columns
//! and pick the richest wording that still fits, so the two never overlap.
//!
//! Note that `str::len()` is a byte count and must not be used for this: `⏱` is three
//! bytes wide but one column, and `🔒` is four bytes but two columns.

use unicode_width::UnicodeWidthStr;

/// Terminal columns `s` occupies, saturating at `u16::MAX`.
pub fn cols(s: &str) -> u16 {
    UnicodeWidthStr::width(s).min(u16::MAX as usize) as u16
}

/// Shortens `s` to at most `max` columns, marking the cut with `…`.
///
/// Cuts on character boundaries and accounts for wide characters, so the result never
/// exceeds `max` columns and never splits a multi-byte character.
pub fn truncate_cols(s: &str, max: u16) -> String {
    if cols(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    // Reserve one column for the ellipsis.
    let budget = max.saturating_sub(1);
    let mut used = 0u16;
    let mut out = String::new();
    for ch in s.chars() {
        let w = cols(ch.encode_utf8(&mut [0u8; 4]));
        if used + w > budget {
            break;
        }
        used += w;
        out.push(ch);
    }
    out.push('…');
    out
}

/// Shortens `s` to at most `max` columns by cutting the MIDDLE, marking the
/// cut with `…` — device names like `/dev/nvme0n1p1` keep their distinctive
/// prefix and suffix. Column- and char-boundary-safe; the byte-slicing
/// predecessor in `util.rs` panicked on non-ASCII names.
pub fn truncate_middle_cols(s: &str, max: u16) -> String {
    if cols(s) <= max {
        return s.to_string();
    }
    if max <= 1 {
        return truncate_cols(s, max);
    }
    // Reserve one column for the ellipsis; split the rest left/right.
    let left_budget = (max - 1) / 2;
    let right_budget = max - 1 - left_budget;

    let mut left_end = 0; // byte index
    let mut used = 0u16;
    for (i, ch) in s.char_indices() {
        let w = cols(ch.encode_utf8(&mut [0u8; 4]));
        if used + w > left_budget {
            break;
        }
        used += w;
        left_end = i + ch.len_utf8();
    }

    let mut right_start = s.len();
    let mut used = 0u16;
    for (i, ch) in s.char_indices().rev() {
        let w = cols(ch.encode_utf8(&mut [0u8; 4]));
        if used + w > right_budget || i < left_end {
            break;
        }
        used += w;
        right_start = i;
    }

    format!("{}…{}", &s[..left_end], &s[right_start..])
}

/// Picks the first (richest) candidate pair that fits side by side in `width` columns
/// with at least `gap` columns between them.
///
/// Candidates are ordered most- to least-detailed; the last one is the floor and is
/// returned even if it does not fit, so callers always get something to render.
pub fn pick_pair<'a>(
    width: u16,
    gap: u16,
    candidates: &[(&'a str, &'a str)],
) -> (&'a str, &'a str) {
    let fits = |left: &str, right: &str| {
        let needed = cols(left)
            .saturating_add(cols(right))
            .saturating_add(if right.is_empty() { 0 } else { gap });
        needed <= width
    };
    for &(left, right) in candidates {
        if fits(left, right) {
            return (left, right);
        }
    }
    candidates.last().copied().unwrap_or(("", ""))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug these helpers exist to prevent: byte length overstates the width of the
    /// glyphs socktop puts in its header, which is what pushed the right-hand text into
    /// the title in the first place.
    #[test]
    fn cols_counts_columns_not_bytes() {
        assert_eq!(cols("abc"), 3);
        // Stopwatch: 3 bytes, 1 column.
        assert_eq!("⏱".len(), 3);
        assert_eq!(cols("⏱"), 1);
        // Lock: 4 bytes, 2 columns.
        assert_eq!("🔒".len(), 4);
        assert_eq!(cols("🔒"), 2);
        assert_eq!(cols("⏱ 500ms metrics | 2000ms procs"), 30);
    }

    #[test]
    fn truncate_respects_the_column_budget() {
        assert_eq!(truncate_cols("cachyos-gaming", 20), "cachyos-gaming");
        assert_eq!(truncate_cols("cachyos-gaming", 14), "cachyos-gaming");
        assert_eq!(truncate_cols("cachyos-gaming", 10), "cachyos-g…");
        assert_eq!(cols(&truncate_cols("cachyos-gaming", 10)), 10);
        assert_eq!(truncate_cols("cachyos-gaming", 1), "…");
        assert_eq!(truncate_cols("cachyos-gaming", 0), "");
    }

    /// Truncation must never land mid-character or overrun the budget on wide glyphs.
    #[test]
    fn truncate_handles_wide_and_multibyte_characters() {
        for max in 0..12u16 {
            let out = truncate_cols("🔒🔒🔒 TLS", max);
            assert!(cols(&out) <= max, "{out:?} exceeds {max} columns");
            assert!(out.chars().all(|c| c != '\u{fffd}'), "{out:?} split a char");
        }
        // A wide glyph that cannot fit beside the ellipsis is dropped whole.
        assert_eq!(truncate_cols("🔒ab", 2), "…");
    }

    /// Middle truncation keeps both ends — the parts that identify a device —
    /// and must never exceed the budget or split a character.
    #[test]
    fn truncate_middle_keeps_both_ends_within_budget() {
        assert_eq!(truncate_middle_cols("/dev/nvme0n1p1", 20), "/dev/nvme0n1p1");
        let out = truncate_middle_cols("/dev/nvme0n1p1", 9);
        assert_eq!(cols(&out), 9);
        assert!(out.starts_with("/dev"), "{out}");
        assert!(out.ends_with("1p1"), "{out}");
        assert!(out.contains('…'), "{out}");
        // Non-ASCII names must not panic (the old byte-slicing version did).
        for max in 0..12u16 {
            let out = truncate_middle_cols("диск-🗄️-данные", max);
            assert!(cols(&out) <= max.max(1), "{out:?} exceeds {max}");
        }
    }

    #[test]
    fn pick_pair_takes_the_richest_that_fits() {
        let candidates = [
            ("full left text", "full right text"),
            ("left text", "right text"),
            ("left", "right"),
        ];
        assert_eq!(pick_pair(80, 2, &candidates), candidates[0]);
        assert_eq!(pick_pair(24, 2, &candidates), candidates[1]);
        assert_eq!(pick_pair(12, 2, &candidates), candidates[2]);
        // Below the floor the last candidate is still returned.
        assert_eq!(pick_pair(1, 2, &candidates), candidates[2]);
    }

    /// The gap is what keeps the two pieces from touching; it must not be charged when
    /// there is no right-hand piece to separate.
    #[test]
    fn pick_pair_only_charges_the_gap_when_both_sides_are_present() {
        let candidates = [("0123456789", "x"), ("0123456789", "")];
        assert_eq!(pick_pair(11, 2, &candidates), candidates[1]);
        assert_eq!(pick_pair(13, 2, &candidates), candidates[0]);
    }
}
