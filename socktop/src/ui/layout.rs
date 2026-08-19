//! Root layout computation, shared by the draw path and the input hit-testing paths.
//!
//! Two modes:
//!
//! * [`LayoutMode::Normal`] — the full layout. CPU graph and per-core bars on top,
//!   Memory over Swap on the left with the GPU panel beside them, then Disks and the
//!   network graphs next to the process table.
//!
//! * [`LayoutMode::Compact`] — entered when the window is too short for the Disks pane
//!   to render even one complete disk card. Disks is dropped, Memory and Swap move side
//!   by side into the space it vacated, the GPU collapses to a single full-width line
//!   (and disappears entirely when the host has no GPU), and every row reclaimed goes to
//!   the CPU graph and per-core bars — which in the fixed layout are squeezed to nothing
//!   long before the rest of the panes stop being useful.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Which of the two layouts [`compute`] produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutMode {
    Normal,
    Compact,
}

impl LayoutMode {
    pub fn is_compact(self) -> bool {
        matches!(self, LayoutMode::Compact)
    }
}

/// Rows the Disks pane needs before it can show one disk card: the card itself is
/// 3 rows (`disks::draw_disks`) plus the pane's own top and bottom border.
const DISKS_MIN_H: u16 = 5;

/// Header line.
const HEADER_H: u16 = 1;
/// Memory and Swap gauges: 1 content row between borders.
const GAUGE_H: u16 = 3;
/// A network graph at its preferred height.
const NET_H: u16 = 5;

// Compact-mode budget. The top row is kept at `TOP_MIN_H` (3 content rows between
// borders) before the network graphs are allowed to shrink, because restoring the CPU
// panes is the entire point of the mode.
const TOP_MIN_H: u16 = 5;
const BOTTOM_PREF_H: u16 = GAUGE_H + 2 * NET_H;
const BOTTOM_MIN_H: u16 = GAUGE_H + 2 * 3;

/// Every pane rect for one frame. `disks` and `gpu` are `None` when the mode omits them.
#[derive(Clone, Copy, Debug)]
pub struct AppLayout {
    pub mode: LayoutMode,
    pub header: Rect,
    pub cpu: Rect,
    pub per_core: Rect,
    pub gpu: Option<Rect>,
    pub mem: Rect,
    pub swap: Rect,
    pub disks: Option<Rect>,
    pub download: Rect,
    pub upload: Rect,
    pub procs: Rect,
}

/// Splits `area` into pane rects.
///
/// `force_compact` comes from `--compact` and pins the compact layout at any size.
/// `has_gpu` decides whether compact mode reserves its one-line GPU strip; it is false
/// until the first metrics payload arrives, so a GPU-less host never reserves the row.
pub fn compute(area: Rect, force_compact: bool, has_gpu: bool) -> AppLayout {
    if force_compact {
        return compact(area, has_gpu);
    }
    let normal = normal(area);
    match normal.disks {
        Some(d) if d.height >= DISKS_MIN_H => normal,
        _ => compact(area, has_gpu),
    }
}

fn split(area: Rect, dir: Direction, constraints: &[Constraint]) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(dir)
        .constraints(constraints)
        .split(area)
}

/// 66/34 split used by every full-width row in the normal layout.
fn left_right(area: Rect) -> std::rc::Rc<[Rect]> {
    split(
        area,
        Direction::Horizontal,
        &[Constraint::Percentage(66), Constraint::Percentage(34)],
    )
}

fn normal(area: Rect) -> AppLayout {
    let rows = split(
        area,
        Direction::Vertical,
        &[
            Constraint::Length(HEADER_H), // header
            Constraint::Ratio(1, 3),      // top row
            Constraint::Length(GAUGE_H),  // memory (left) + GPU (right, part 1)
            Constraint::Length(GAUGE_H),  // swap (left)   + GPU (right, part 2)
            Constraint::Min(2 * NET_H),   // bottom: disks + net (left), top procs (right)
        ],
    );

    let top = left_right(rows[1]);
    let mem_lr = left_right(rows[2]);
    let swap_lr = left_right(rows[3]);

    // GPU spans the same vertical space as Memory + Swap.
    let gpu = Rect {
        x: mem_lr[1].x,
        y: mem_lr[1].y,
        width: mem_lr[1].width,
        height: mem_lr[1].height + swap_lr[1].height,
    };

    let bottom = split(
        rows[4],
        Direction::Horizontal,
        &[Constraint::Percentage(60), Constraint::Percentage(40)],
    );
    let left_stack = split(
        bottom[0],
        Direction::Vertical,
        &[
            Constraint::Min(4),        // disks absorbs the slack
            Constraint::Length(NET_H), // download
            Constraint::Length(NET_H), // upload
        ],
    );

    AppLayout {
        mode: LayoutMode::Normal,
        header: rows[0],
        cpu: top[0],
        per_core: top[1],
        gpu: Some(gpu),
        mem: mem_lr[0],
        swap: swap_lr[0],
        disks: Some(left_stack[0]),
        download: left_stack[1],
        upload: left_stack[2],
        procs: bottom[1],
    }
}

fn compact(area: Rect, has_gpu: bool) -> AppLayout {
    let gpu_h = if has_gpu { GAUGE_H } else { 0 };
    let avail = area.height.saturating_sub(HEADER_H + gpu_h);

    // Give the top row its floor first, then share any surplus with the bottom so the
    // process table keeps growing with the window instead of staying pinned at 13 rows.
    let (top_h, bottom_h) = if avail >= TOP_MIN_H + BOTTOM_PREF_H {
        let top = TOP_MIN_H + (avail - TOP_MIN_H - BOTTOM_PREF_H) / 2;
        (top, avail - top)
    } else if avail >= TOP_MIN_H + BOTTOM_MIN_H {
        (TOP_MIN_H, avail - TOP_MIN_H)
    } else {
        // Smaller than both floors: the network graphs are already at their minimum, so
        // the top row takes what is left (panes clip below this point).
        let bottom = BOTTOM_MIN_H.min(avail);
        (avail - bottom, bottom)
    };

    let rows = split(
        area,
        Direction::Vertical,
        &[
            Constraint::Length(HEADER_H),
            Constraint::Length(top_h),
            Constraint::Length(gpu_h),
            Constraint::Length(bottom_h),
        ],
    );

    let top = left_right(rows[1]);

    let bottom = split(
        rows[3],
        Direction::Horizontal,
        &[Constraint::Percentage(60), Constraint::Percentage(40)],
    );
    // Memory + Swap take the row Disks used to occupy; the graphs share what is left.
    let left_stack = split(
        bottom[0],
        Direction::Vertical,
        &[
            Constraint::Length(GAUGE_H),
            Constraint::Fill(1),
            Constraint::Fill(1),
        ],
    );
    let gauges = split(
        left_stack[0],
        Direction::Horizontal,
        &[Constraint::Percentage(50), Constraint::Percentage(50)],
    );

    AppLayout {
        mode: LayoutMode::Compact,
        header: rows[0],
        cpu: top[0],
        per_core: top[1],
        gpu: has_gpu.then_some(rows[2]),
        mem: gauges[0],
        swap: gauges[1],
        disks: None,
        download: left_stack[1],
        upload: left_stack[2],
        procs: bottom[1],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    /// The height where the normal layout still fits a full disk card. Below it the CPU
    /// panes are the ones that collapse, which is what compact mode exists to prevent.
    #[test]
    fn tall_window_stays_normal() {
        let l = compute(area(120, 40), false, true);
        assert_eq!(l.mode, LayoutMode::Normal);
        assert!(l.disks.expect("disks pane").height >= DISKS_MIN_H);
    }

    #[test]
    fn short_window_switches_to_compact() {
        let l = compute(area(120, 24), false, true);
        assert_eq!(l.mode, LayoutMode::Compact);
        assert!(l.disks.is_none());
    }

    /// The switch happens exactly when Disks can no longer show one card, and never
    /// oscillates: every height above the crossover is normal, every height below is
    /// compact.
    #[test]
    fn mode_is_monotonic_in_height() {
        let mut first_normal = None;
        for h in 10..=60u16 {
            let mode = compute(area(120, h), false, true).mode;
            match (mode, first_normal) {
                (LayoutMode::Normal, None) => first_normal = Some(h),
                (LayoutMode::Compact, Some(prev)) => {
                    panic!("height {h} went back to compact after normal at {prev}")
                }
                _ => {}
            }
        }
        assert!(first_normal.is_some(), "never reached the normal layout");
    }

    #[test]
    fn force_compact_overrides_a_tall_window() {
        let l = compute(area(200, 80), true, true);
        assert_eq!(l.mode, LayoutMode::Compact);
        assert!(l.disks.is_none());
    }

    #[test]
    fn compact_drops_the_gpu_row_without_a_gpu() {
        let with = compute(area(120, 24), true, true);
        let without = compute(area(120, 24), true, false);
        assert!(with.gpu.is_some());
        assert_eq!(with.gpu.expect("gpu strip").height, GAUGE_H);
        assert!(without.gpu.is_none());
        // The rows a GPU-less host saves are shared between the CPU panes and the
        // bottom half, and none of them are left as a gap.
        assert!(without.cpu.height > with.cpu.height);
        assert!(without.procs.height > with.procs.height);
        assert_eq!(without.procs.y + without.procs.height, 24);
    }

    /// Compact exists to keep the CPU graph and per-core bars drawable: both need
    /// content rows inside their borders.
    #[test]
    fn compact_keeps_the_cpu_panes_drawable() {
        for h in 18..=32u16 {
            let l = compute(area(120, h), false, true);
            assert_eq!(l.mode, LayoutMode::Compact, "height {h}");
            assert!(
                l.cpu.height >= TOP_MIN_H,
                "height {h}: cpu pane only {} rows",
                l.cpu.height
            );
            assert_eq!(l.per_core.height, l.cpu.height);
        }
    }

    /// Regression guard for the bug this mode fixes: at 18 rows the old fixed layout
    /// left the top row with no drawable interior at all.
    #[test]
    fn compact_beats_the_fixed_layout_at_18_rows() {
        let compact = compute(area(120, 18), false, true);
        let fixed = normal(area(120, 18));
        assert!(fixed.cpu.height <= 2, "fixed layout unexpectedly usable");
        assert!(compact.cpu.height > fixed.cpu.height);
    }

    #[test]
    fn compact_panes_tile_the_area_without_gaps() {
        for h in 16..=32u16 {
            for has_gpu in [true, false] {
                let l = compute(area(120, h), true, has_gpu);
                assert_eq!(l.header.y, 0);
                assert_eq!(l.cpu.y, l.header.y + l.header.height);
                assert_eq!(l.per_core.x, l.cpu.x + l.cpu.width);

                let after_cpu = l.cpu.y + l.cpu.height;
                let bottom_y = match l.gpu {
                    Some(g) => {
                        assert_eq!(g.y, after_cpu);
                        assert_eq!(g.width, 120, "gpu strip spans the full width");
                        g.y + g.height
                    }
                    None => after_cpu,
                };
                assert_eq!(l.mem.y, bottom_y);
                // Memory and Swap sit side by side on one row.
                assert_eq!(l.swap.y, l.mem.y);
                assert_eq!(l.swap.x, l.mem.x + l.mem.width);
                assert_eq!(l.mem.height, GAUGE_H);
                assert_eq!(l.download.y, l.mem.y + l.mem.height);
                assert_eq!(l.upload.y, l.download.y + l.download.height);
                assert_eq!(l.procs.y, bottom_y);
            }
        }
    }

    /// A degenerate size must not panic or produce rects outside the frame.
    #[test]
    fn tiny_windows_stay_inside_the_frame() {
        for h in 0..=16u16 {
            for w in [0u16, 1, 20, 80] {
                let l = compute(area(w, h), false, true);
                for r in [l.header, l.cpu, l.per_core, l.mem, l.swap, l.procs] {
                    assert!(r.y + r.height <= h, "{r:?} escapes height {h}");
                    assert!(r.x + r.width <= w, "{r:?} escapes width {w}");
                }
            }
        }
    }
}
