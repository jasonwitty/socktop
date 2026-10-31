//! Small UI helpers: human-readable sizes, truncation, icons.

/// Byte count as `B / KB / MB / GB / TB`. Kept as a ladder so the two
/// precisions can never drift apart in where they switch units.
fn human_fmt(b: u64, mb_prec: usize, gb_prec: usize, tb_prec: usize) -> String {
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
        return format!("{mb:.mb_prec$}MB");
    }
    let gb = mb / K;
    if gb < K {
        return format!("{gb:.gb_prec$}GB");
    }
    let tb = gb / K;
    format!("{tb:.tb_prec$}TB")
}

/// Default precision — the right amount for tables, disks and rates, where
/// every extra column costs layout width.
pub fn human(b: u64) -> String {
    human_fmt(b, 1, 1, 2)
}

/// One digit finer, used by the memory and swap gauges. btop reports RAM at
/// this precision, so the two can be compared digit by digit, and the last
/// digit visibly ticks between refreshes instead of the bar looking frozen.
pub fn human_precise(b: u64) -> String {
    human_fmt(b, 2, 2, 2)
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
mod tests {
    use super::*;

    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    const TIB: u64 = 1024 * GIB;

    #[test]
    fn both_precisions_switch_units_at_the_same_place() {
        for b in [0u64, 999, KIB, 1023 * KIB, MIB, GIB - 1, GIB, TIB - 1, TIB] {
            let unit = |s: String| -> String {
                s.chars()
                    .filter(|c| !c.is_ascii_digit() && *c != '.')
                    .collect()
            };
            assert_eq!(
                unit(human(b)),
                unit(human_precise(b)),
                "unit changed for {b}"
            );
        }
    }

    #[test]
    fn precise_adds_exactly_one_digit_above_megabytes() {
        // 9.4 GiB: the value the memory gauge shows.
        assert_eq!(human(9_626 * MIB), "9.4GB");
        assert_eq!(human_precise(9_626 * MIB), "9.40GB");
        assert_eq!(human(512 * MIB), "512.0MB");
        assert_eq!(human_precise(512 * MIB), "512.00MB");
        // Kilobytes and bytes gain nothing useful, so they stay put.
        assert_eq!(human_precise(512 * KIB), "512.0KB");
        assert_eq!(human_precise(999), "999B");
    }

    #[test]
    fn precise_separates_values_the_coarse_form_merges() {
        // The point of the extra digit: two adjacent refreshes used to render
        // identically even though the agent reported different numbers.
        let a = 9_948_000_000;
        let b = 9_952_000_000;
        assert_eq!(human(a), human(b));
        assert_ne!(human_precise(a), human_precise(b));
    }

    #[test]
    fn terabytes_keep_two_digits_in_both() {
        assert_eq!(human(3 * TIB), "3.00TB");
        assert_eq!(human_precise(3 * TIB + TIB / 8), "3.12TB");
    }
}
