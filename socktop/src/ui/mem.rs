//! Memory gauge.
//!
//! Three stacked segments rather than one flat bar, because the number in the
//! process list and the number in the bar answer different questions:
//!
//! ```text
//!   used  = MemTotal - MemAvailable   ← the fraction of the bar's first slice
//!   cache = MemAvailable - MemFree    ← reclaimable page + buffer cache
//!   free  = MemFree
//! ```
//!
//! They add up to `MemTotal` by construction. The cache segment is what
//! explains a complaint that would otherwise look like a bug: a process can
//! report gigabytes of RSS while the bar barely moves. llama.cpp maps its GGUF
//! model by default, and clean file-backed pages are reclaimable, so the
//! kernel counts them in `MemAvailable` (not "used") while they *are* in the
//! process's RSS. Anonymous memory — heap, stacks, llama.cpp run with
//! `--no-mmap` or `--mlock` — does move the bar. See
//! `scripts/mem_accounting_check.sh`, which demonstrates both cases against a
//! live agent.
//!
//! Agents that predate these fields send no `mem_free`/`mem_available`, and
//! Windows has no separate free figure (sysinfo reports free == available). In
//! both cases `cache` is 0 and the bar degrades to used + remainder, i.e. how
//! it used to look.

use crate::types::Metrics;
use crate::ui::theme;
use crate::ui::util::human_precise;
use ratatui::layout::Alignment;
use ratatui::{
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Block, Borders, Gauge},
};

/// The three parts of the bar, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemSplit {
    pub used: u64,
    pub cache: u64,
    pub free: u64,
}

/// Split a metrics snapshot into used / cache / free.
///
/// `cache` is derived rather than sent: `MemAvailable - MemFree` is the
/// kernel's own reclaimable figure (it differs by a whisker from
/// `Buffers + Cached + SReclaimable - Shmem` because `MemAvailable` also
/// holds back the lowmem watermarks), and is what sysinfo itself falls back to
/// on platforms without `MemAvailable`. Where free is not reported at all this
/// is 0, so the bar just shows used plus remainder.
pub fn split_memory(m: &Metrics) -> MemSplit {
    MemSplit {
        used: m.mem_used,
        cache: m.mem_available.saturating_sub(m.mem_free),
        free: m.mem_free,
    }
}

/// Column widths of the used / cache / free segments across `width` cells.
///
/// The three always add up to `width`. A non-zero value that would round to
/// zero columns still gets one cell, so small-but-real memory is visible.
pub fn segment_widths(total: u64, used: u64, cache: u64, width: u16) -> (u16, u16, u16) {
    if width == 0 {
        return (0, 0, 0);
    }
    if total == 0 {
        return (0, 0, width);
    }
    let w = u128::from(width);
    let t = u128::from(total);
    let used = u128::from(used).min(t);
    let cache = u128::from(cache).min(t - used);

    let mut used_w = (used * w / t) as u16;
    if used > 0 && used_w == 0 {
        used_w = 1;
    }
    let mut cache_w = ((cache * w / t) as u16).min(width - used_w);
    if cache > 0 && cache_w == 0 {
        cache_w = (width - used_w).min(1);
    }
    (used_w, cache_w, width.saturating_sub(used_w + cache_w))
}

pub fn draw_mem(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    let block = || Block::default().borders(Borders::ALL).title("Memory");

    let Some(mm) = m else {
        f.render_widget(
            Gauge::default()
                .block(block())
                .gauge_style(Style::default().fg(theme::MEM_USED))
                .percent(0)
                .label(""),
            area,
        );
        return;
    };

    let total = mm.mem_total;
    let s = split_memory(mm);

    // The used/total summary rides the top border: unlike a label inside the
    // bar it is readable however small the segments get. It is dropped (and
    // the used segment is labelled instead) when the pane is too narrow.
    let summary = format!("{} / {}", human_precise(mm.mem_used), human_precise(total));
    let summary_in_title = total > 0
        && usize::from(area.width) >= 3 + "Memory".chars().count() + summary.chars().count();

    let mut b = block();
    if summary_in_title {
        b = b.title(Line::from(summary.clone()).alignment(Alignment::Right));
    }
    let inner = b.inner(area);
    f.render_widget(b, area);

    let (uw, cw, fw) = segment_widths(total, s.used, s.cache, inner.width);
    let seg = |x: u16, w: u16| Rect {
        x,
        y: inner.y,
        width: w,
        height: inner.height,
    };

    // used
    if uw > 0 {
        let label = human_precise(s.used);
        let show = !summary_in_title && usize::from(uw) >= label.chars().count();
        f.render_widget(
            Gauge::default()
                .gauge_style(Style::default().fg(theme::MEM_USED))
                .percent(100)
                .label(if show { label } else { String::new() }),
            seg(inner.x, uw),
        );
    }

    // cache (reclaimable) — the llama.cpp/model-file/`free -h` buff-cache slice
    if cw > 0 {
        let label = format!("cache {}", human_precise(s.cache));
        let show = usize::from(cw) >= label.chars().count();
        f.render_widget(
            Gauge::default()
                .gauge_style(Style::default().fg(theme::MEM_CACHE))
                .percent(100)
                .label(if show { label } else { String::new() }),
            seg(inner.x + uw, cw),
        );
    }

    // free / unknown remainder
    if fw > 0 {
        let label = format!("free {}", human_precise(s.free));
        let show = s.free > 0 && usize::from(fw) >= label.chars().count();
        f.render_widget(
            Gauge::default()
                .gauge_style(Style::default().fg(theme::MEM_FREE))
                .percent(100)
                .label(if show { label } else { String::new() }),
            seg(inner.x + uw + cw, fw),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const MIB: u64 = 1 << 20;
    // The shape seen on the 46 GiB box while an 8 GiB heap and a 1.6 GiB
    // mmap'd ISO were live (scripts/mem_accounting_check.sh): only 9.4 GiB is
    // "used", 28.1 GiB is reclaimable cache.
    const TOTAL: u64 = 46 * 1024 * MIB;
    const USED: u64 = 9_626 * MIB;
    const CACHE: u64 = 28_774 * MIB;
    const FREE: u64 = TOTAL - USED - CACHE;

    fn metrics(total: u64, used: u64, free: u64, available: u64) -> Metrics {
        Metrics {
            sampled_at_ms: None,
            cpu_total: 0.0,
            cpu_per_core: vec![],
            mem_total: total,
            mem_used: used,
            mem_free: free,
            mem_available: available,
            swap_total: 0,
            swap_used: 0,
            hostname: "t".into(),
            cpu_temp_c: None,
            disks: vec![],
            networks: vec![],
            top_processes: vec![],
            gpus: None,
            gpus_extra: None,
            process_count: Some(0),
        }
    }

    /// A full report: the agent gave us free and available (Linux, macOS,
    /// FreeBSD). Windows also sends both, just with free == available.
    fn full_metrics() -> Metrics {
        metrics(TOTAL, USED, FREE, FREE + CACHE)
    }

    #[test]
    fn segments_add_up_to_total() {
        let s = split_memory(&full_metrics());
        assert_eq!(
            s,
            MemSplit {
                used: USED,
                cache: CACHE,
                free: FREE
            }
        );
        assert_eq!(s.used + s.cache + s.free, TOTAL);
    }

    #[test]
    fn old_agent_has_no_cache_segment() {
        // Old agents send neither field; serde defaults them to 0. The bar must
        // fall back to used + remainder instead of inventing a cache slice, and
        // must not claim there is no free memory.
        let s = split_memory(&metrics(TOTAL, USED, 0, 0));
        assert_eq!((s.cache, s.free), (0, 0));
        assert_eq!(s.used, USED);
    }

    #[test]
    fn widths_always_fill_the_bar() {
        for width in [0u16, 1, 2, 3, 10, 22, 40, 58, 120] {
            let (u, c, fr) = segment_widths(TOTAL, USED, CACHE, width);
            assert_eq!(u + c + fr, width, "width {width}");
            assert!(u <= width && c <= width && fr <= width, "width {width}");
        }
    }

    #[test]
    fn tiny_values_still_get_one_cell() {
        assert_eq!(segment_widths(TOTAL, MIB, MIB, 40), (1, 1, 38));
        assert_eq!(segment_widths(TOTAL, MIB, 0, 1), (1, 0, 0));
    }

    #[test]
    fn zero_total_leaves_the_bar_empty() {
        assert_eq!(segment_widths(0, 0, 0, 40), (0, 0, 40));
    }

    #[test]
    fn hostile_values_cannot_overflow_the_bar() {
        // used > total (old agents round `used` differently; a buggy agent
        // could report anything): clamp, never wrap or overdraw.
        assert_eq!(segment_widths(TOTAL, u64::MAX, u64::MAX, 40), (40, 0, 0));
    }

    /// Cells painted in each segment colour on the gauge row. Gauge renders a
    /// label by swapping fg/bg, so a segment cell matches on either.
    fn count(width: u16, m: &Metrics) -> (usize, usize, usize) {
        let mut terminal = Terminal::new(TestBackend::new(width, 3)).unwrap();
        terminal
            .draw(|f| draw_mem(f, Rect::new(0, 0, width, 3), Some(m)))
            .unwrap();
        let buf = terminal.backend().buffer();
        let row = buf.area().bottom() - 2; // gauge row = inner row 1
        let n = |color| {
            (1..width - 1)
                .filter(|x| {
                    let s = buf[(*x, row)].style();
                    s.fg == Some(color) || s.bg == Some(color)
                })
                .count()
        };
        (n(theme::MEM_USED), n(theme::MEM_CACHE), n(theme::MEM_FREE))
    }

    fn render(width: u16, m: &Metrics) -> (String, String) {
        let mut terminal = Terminal::new(TestBackend::new(width, 3)).unwrap();
        terminal
            .draw(|f| draw_mem(f, Rect::new(0, 0, width, 3), Some(m)))
            .unwrap();
        let buf = terminal.backend().buffer();
        let line = |y: u16| -> String {
            (0..width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect()
        };
        (line(0), line(1))
    }

    #[test]
    fn all_three_segments_render() {
        let (u, c, fr) = count(60, &full_metrics());
        assert!(u > 0 && c > 0 && fr > 0, "used={u} cache={c} free={fr}");
        assert_eq!(u + c + fr, 58, "inner width of a 60-wide bordered pane");
        // cache is nearly two thirds of the machine and must look like it
        assert!(c > fr + u, "used={u} cache={c} free={fr}");
    }

    #[test]
    fn cache_segment_survives_a_narrow_pane() {
        // Compact layout puts Memory and Swap side by side. The cache slice is
        // the whole point, so it must not be the first thing to disappear.
        let (u, c, fr) = count(24, &full_metrics());
        assert!(u > 0 && c > 0 && fr > 0, "used={u} cache={c} free={fr}");
        assert_eq!(u + c + fr, 22);
    }

    #[test]
    fn bar_is_labelled_and_summarised() {
        let (title, bar) = render(60, &full_metrics());
        assert!(title.contains("Memory"), "{title}");
        assert!(
            title.contains("9.40GB / 46.00GB"),
            "summary missing: {title}"
        );
        assert!(bar.contains("cache 28.10GB"), "no cache label: {bar}");
        assert!(bar.contains("free 8.50GB"), "no free label: {bar}");
        assert!(!bar.contains("cache 28.10GBfree"), "labels collided: {bar}");
    }

    #[test]
    fn old_agent_keeps_the_old_look() {
        let m = metrics(TOTAL, USED, 0, 0);
        let (u, c, fr) = count(60, &m);
        assert!(u > 0, "no used bar at all");
        assert_eq!(c, 0, "invented a cache segment for an old agent");
        assert_eq!(u + c + fr, 58);
        let (title, bar) = render(60, &m);
        assert!(title.contains("9.40GB / 46.00GB"), "{title}");
        assert!(
            !bar.contains("free "),
            "claimed a free figure it does not have: {bar}"
        );
    }

    #[test]
    fn no_metrics_renders_an_empty_bar() {
        let mut terminal = Terminal::new(TestBackend::new(40, 3)).unwrap();
        terminal
            .draw(|f| draw_mem(f, Rect::new(0, 0, 40, 3), None))
            .unwrap();
        let buf = terminal.backend().buffer();
        let bar: String = (0..40).map(|x| buf[(x, 1)].symbol().to_string()).collect();
        assert!(bar.contains("Memory") || !bar.contains('█'), "{bar}");
    }
}
