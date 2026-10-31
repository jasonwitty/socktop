//! GPU pane.
//!
//! One card gets the rows it has always had: name, utilisation, VRAM — plus a
//! temperature row when the driver reports one. More than one card gets a tab
//! strip whose first tab aggregates the whole machine:
//!
//! ```text
//! ┌ GPU (2 cards · 5.00GB/48.00GB) ─────────────────────┐
//! │  All | 0:RTX 3090 | 1:RTX 3090                     │
//! │  ██████████░░░░░░░░░░  max: 87%                     │
//! │  ███░░░░░░░░░░░░░░░░░  vram: 5.00GB/48.00GB         │
//! │  0 87% 4.0GB 71°   1 3% 1.0GB 68°                   │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! # Why the aggregate is not an average
//!
//! NVML reports `utilization` as a per-device duty cycle over a sample window,
//! not a throughput measure, so "both cards at 50%" and "one saturated, one
//! idle" arrive as the same pair of numbers. Averaging across cards throws away
//! the only distinction the pane exists to show. So the aggregate bar is the
//! **maximum** across cards and the line under it keeps every card's own
//! numbers. VRAM and power do sum — bytes and watts really are additive, and
//! total VRAM is what tells you a layer failed to fit.
//!
//! # Where the cards come from
//!
//! Agents report the primary card in `gpus` and any others in `gpus_extra`;
//! [`gpu_set`] recombines them, primary first, one entry per physical card.
//! Agents that predate multi-GPU send no `gpus_extra`, and the pane behaves
//! exactly as it always did: one card, no strip. Temperatures, power and fan
//! speed are equally optional — a driver that will not answer for them leaves
//! the row out rather than showing a zero.

use crate::types::Metrics;
use crate::ui::theme;
use crate::ui::util::{human, human_precise};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Tabs},
};
use socktop_connector::GpuInfo;
use unicode_width::UnicodeWidthStr;

/// Tab 0 aggregates every card, whenever there is more than one card to
/// aggregate. With a single card tab 0 *is* that card and there is no strip.
pub const TAB_ALL: usize = 0;

/// Every card the agent reported: `gpus` first, then `gpus_extra`, never the
/// same card twice.
///
/// Identity is the UUID when the agent supplied one — the only key that survives
/// a card moving between driver indexes — and name + VRAM size otherwise, which
/// is enough to catch an agent that reports its primary card through the display
/// driver and again through NVML.
pub fn gpu_set(metrics: Option<&Metrics>) -> Vec<GpuInfo> {
    let Some(m) = metrics else {
        return Vec::new();
    };
    let mut out: Vec<GpuInfo> = Vec::new();
    for list in [m.gpus.as_ref(), m.gpus_extra.as_ref()]
        .into_iter()
        .flatten()
    {
        for g in list {
            if !out.iter().any(|seen| same_card(seen, g)) {
                out.push(g.clone());
            }
        }
    }
    out
}

fn same_card(a: &GpuInfo, b: &GpuInfo) -> bool {
    match (a.uuid.as_deref(), b.uuid.as_deref()) {
        (Some(x), Some(y)) => x == y,
        // A UUID on one side only: the unlabelled card may well *be* the
        // labelled one, so fall back to the weak key rather than trusting the
        // absence of a field.
        _ => a.name == b.name && a.mem_total.is_some() && a.mem_total == b.mem_total,
    }
}

/// How many tabs the pane has for this card list.
pub fn tab_count(cards: &[GpuInfo]) -> usize {
    if cards.len() > 1 { cards.len() + 1 } else { 1 }
}

/// Clamp a saved selection to the current frame. Cards can disappear between
/// refreshes (driver reload, agent restart), and a selection pointing past the
/// end has to move rather than read past the end of the list.
pub fn clamp_tab(sel: &mut usize, cards: &[GpuInfo]) {
    let tabs = tab_count(cards);
    if *sel >= tabs {
        *sel = tabs - 1;
    }
}

/// Cycle the selection: `step` of 1 goes forward, `tabs - 1` goes back.
pub fn cycle_tab(sel: &mut usize, cards: &[GpuInfo], step: usize) {
    let tabs = tab_count(cards);
    if tabs > 1 {
        *sel = (*sel + step) % tabs;
    }
}

/// Machine-wide figures. See the module docs for why utilisation is a maximum
/// while VRAM and power are sums.
#[derive(Debug, Clone, PartialEq)]
pub struct GpuTotals {
    pub count: usize,
    pub vram_used: u64,
    pub vram_total: u64,
    pub util_max: f32,
    pub power_w: f32,
    /// Hottest card, because that is the one worth acting on.
    pub temp_max: Option<f32>,
    /// The *lowest* slowdown limit among the cards: whichever throttles first,
    /// and the number the hottest reading should be judged against.
    pub temp_slowdown_c: Option<f32>,
}

pub fn aggregate(cards: &[GpuInfo]) -> GpuTotals {
    let mut t = GpuTotals {
        count: cards.len(),
        vram_used: 0,
        vram_total: 0,
        util_max: 0.0,
        power_w: 0.0,
        temp_max: None,
        temp_slowdown_c: None,
    };
    for c in cards {
        t.vram_used = t.vram_used.saturating_add(c.mem_used.unwrap_or(0));
        t.vram_total = t.vram_total.saturating_add(c.mem_total.unwrap_or(0));
        if let Some(u) = c.utilization {
            t.util_max = t.util_max.max(u.clamp(0.0, 100.0));
        }
        if let Some(p) = c.power_w {
            t.power_w += p;
        }
        if let Some(tp) = c.temp {
            t.temp_max = Some(t.temp_max.map_or(tp, |x| x.max(tp)));
        }
        if let Some(sl) = c.temp_slowdown_c {
            t.temp_slowdown_c = Some(t.temp_slowdown_c.map_or(sl, |x| x.min(sl)));
        }
    }
    t
}

/// Device name without the marketing prefix: `NVIDIA GeForce RTX 3090` becomes
/// `RTX 3090`. Tab labels and pane titles have a couple of dozen columns.
pub fn short_name(g: &GpuInfo) -> String {
    let mut s = g.name.as_deref().unwrap_or("").trim();
    while let Some(next) = ["NVIDIA", "GeForce", "AMD", "ATI", "Radeon", "Intel"]
        .iter()
        .find_map(|p| s.strip_prefix(p).map(str::trim_start))
        // Never strip a name to nothing: a card called "Intel" stays "Intel".
        .filter(|next| !next.is_empty())
    {
        s = next;
    }
    // A driver that answers with an empty string still needs a label.
    if s.is_empty() { "GPU" } else { s }.to_string()
}

/// Health glyph for a GPU temperature.
///
/// Deliberately not the CPU pane's ladder: a 3090 sits at 71°C while working,
/// which the CPU thresholds would call a warning. When the driver reports the
/// card's own slowdown point the glyph is graded against that instead, so 90°C
/// against a 93°C limit warns and 90°C against a 105°C limit does not.
fn temp_icon(t: f32, slowdown: Option<f32>) -> &'static str {
    match slowdown {
        Some(sl) if sl > 0.0 => {
            let used = t / sl;
            if used < 0.85 {
                "😎"
            } else if used < 0.95 {
                "⚠️"
            } else {
                "🔥"
            }
        }
        _ => {
            if t < 70.0 {
                "😎"
            } else if t < 88.0 {
                "⚠️"
            } else {
                "🔥"
            }
        }
    }
}
/// The tab strip's labels, most detailed first as width runs out.
///
/// `All` is never dropped and neither is any card: N cards always produce N + 1
/// labels, so the last resort is bare indexes. A card that quietly disappears
/// from the strip is indistinguishable from a card that went idle.
pub fn tab_labels(cards: &[GpuInfo], width: u16) -> Vec<String> {
    if cards.len() <= 1 {
        return Vec::new();
    }
    // `Tabs` draws each label as ` label ` with a one-column divider between, and
    // the aggregate tab is always present too, so it has to be charged for as
    // well — otherwise a strip can be judged to fit and then get clipped.
    let fits = |labels: &[String]| -> bool {
        let sum: usize =
            labels.iter().map(|l| l.width() + 2).sum::<usize>() + 2 + "All".width() + labels.len();
        sum <= usize::from(width)
    };
    let with_all = |labels: Vec<String>| {
        let mut out = Vec::with_capacity(labels.len() + 1);
        out.push("All".into());
        out.extend(labels);
        out
    };

    let named: Vec<String> = cards
        .iter()
        .enumerate()
        .map(|(i, g)| format!("{i}:{}", short_name(g)))
        .collect();
    if fits(&named) {
        return with_all(named);
    }
    // Index plus the last word of the model name ("RTX 3090" -> "3090").
    let tails: Vec<String> = cards
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let name = short_name(g);
            format!("{i}:{}", name.split(' ').next_back().unwrap_or(&name))
        })
        .collect();
    if fits(&tails) {
        return with_all(tails);
    }
    with_all(
        cards
            .iter()
            .enumerate()
            .map(|(i, _)| i.to_string())
            .collect(),
    )
}

/// Where each label is painted, mirroring `Tabs`' own layout (` label ` with a
/// one-column divider) so a click lands on the card it looks like it is on.
fn tab_rects(area: Rect, labels: &[String]) -> Vec<Rect> {
    let mut out = Vec::with_capacity(labels.len());
    let mut x = area.x;
    for label in labels {
        let left = x + 1; // left padding
        let w = label.width() as u16;
        if left + w <= area.right() {
            out.push(Rect {
                x: left,
                y: area.y,
                width: w,
                height: 1,
            });
        }
        // right padding + divider
        x = left + w + 2;
    }
    out
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Row {
    /// Tab strip, when there is more than one card.
    Strip,
    /// Full device name, when there is only one (the short form is in the title).
    Name,
    Util,
    Vram,
    /// Temperatures, power, fan speed.
    Sensors,
}

/// Which rows fit in `h` inner rows, in the order they are painted. `head` is
/// the tab strip for several cards, the device name for one.
///
/// Space is given up in this order: sensor row first, then the head row. The two
/// bars stay — a GPU pane without utilisation and VRAM is not worth drawing.
/// Losing the strip does not lose a card: the title always carries the card
/// count when there is more than one.
fn row_plan(h: u16, head: Row, with_sensors: bool) -> Vec<Row> {
    let mut rows = vec![Row::Util, Row::Vram, head];
    if with_sensors {
        rows.push(Row::Sensors);
    }
    while rows.len() > usize::from(h) {
        if let Some(at) = rows.iter().position(|r| *r == Row::Sensors) {
            rows.remove(at);
        } else if let Some(at) = rows.iter().position(|r| *r == head) {
            rows.remove(at);
        } else {
            break;
        }
    }
    // Paint order, not priority order.
    rows.sort_by_key(|r| match r {
        Row::Strip => 0,
        Row::Name => 1,
        Row::Util => 2,
        Row::Vram => 3,
        Row::Sensors => 4,
    });
    rows
}

/// Pick the most detailed candidate that fits `width`. Candidates are given
/// most-detailed first, so falling back to the last is the "no room at all"
/// answer rather than an arbitrary member of the list.
fn fit(candidates: &[String], width: usize) -> Option<String> {
    candidates
        .iter()
        .find(|c| c.width() <= width)
        .cloned()
        .or_else(|| candidates.last().cloned())
}

/// Per-card utilisation / VRAM / temperature line for the aggregate tab.
fn per_card_line(cards: &[GpuInfo], width: usize) -> Option<String> {
    let build = |vram: bool, temp: bool| -> String {
        cards
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let mut s = format!(
                    "{} {:.0}%",
                    i,
                    g.utilization.unwrap_or(0.0).clamp(0.0, 100.0)
                );
                if vram {
                    s.push_str(&format!(" {}", human(g.mem_used.unwrap_or(0))));
                }
                if let (true, Some(t)) = (temp, g.temp) {
                    s.push_str(&format!(" {:.0}°", t));
                }
                s
            })
            .collect::<Vec<_>>()
            .join("   ")
    };
    fit(
        &[build(true, true), build(true, false), build(false, true)],
        width,
    )
}

/// The sensor row for one card: temperature, power and fan, richest form that
/// fits `width`. `None` when the driver answered none of them, so the row is
/// left out entirely rather than rendered as a line of dashes.
///
/// What has to survive the narrowest pane is the temperature and its glyph: the
/// glyph is graded against this card's own slowdown limit, so it answers "is this
/// card too hot" without the reader knowing the model. The VRAM limit is the first
/// thing dropped — NVML publishes it without a live memory reading, so it is
/// context rather than a gauge.
fn sensor_line(g: &GpuInfo, width: usize) -> Option<String> {
    let t = g.temp?;
    let icon = temp_icon(t, g.temp_slowdown_c);
    let temp_labelled = format!("temp: {t:.1}°C {icon}");
    let temp_short = format!("{t:.0}°C {icon}");
    let temp_bare = format!("{t:.0}°C");
    // The limit the glyph is graded against, kept next to the temperature.
    let die_limit = g
        .temp_slowdown_c
        .map(|l| format!("limit {l:.0}°C"))
        .unwrap_or_default();
    let power_pair = match (g.power_w, g.power_limit_w) {
        (Some(p), Some(l)) => format!("power: {p:.0}/{l:.0}W"),
        (Some(p), None) => format!("power: {p:.0}W"),
        (None, _) => String::new(),
    };
    let power_short = g
        .power_w
        .map(|p| match g.power_limit_w {
            Some(l) => format!("{p:.0}/{l:.0}W"),
            None => format!("{p:.0}W"),
        })
        .unwrap_or_default();
    // A stopped or passive fan is an error from NVML, not 0%, so absence stays
    // absent rather than claiming the fan is dead.
    let fan = g.fan_pct.map(|f| format!("fan {f}%")).unwrap_or_default();
    let vram_limit = g
        .temp_mem_max_c
        .map(|t| format!("vram max {t:.0}°C"))
        .unwrap_or_default();

    let join = |parts: [&str; 5]| -> Option<String> {
        let kept: Vec<&str> = parts.into_iter().filter(|p| !p.is_empty()).collect();
        (!kept.is_empty()).then(|| kept.join("  "))
    };

    let candidates: Vec<String> = [
        // Everything, with labels.
        join([
            temp_labelled.as_str(),
            die_limit.as_str(),
            power_pair.as_str(),
            fan.as_str(),
            vram_limit.as_str(),
        ]),
        // Drop the VRAM limit before anything that is a live reading.
        join([
            temp_labelled.as_str(),
            die_limit.as_str(),
            power_pair.as_str(),
            fan.as_str(),
            "",
        ]),
        join([
            temp_labelled.as_str(),
            die_limit.as_str(),
            power_pair.as_str(),
            "",
            "",
        ]),
        // Keep the glyph and the limit it is graded against; drop the labels.
        join([
            temp_short.as_str(),
            die_limit.as_str(),
            power_pair.as_str(),
            "",
            "",
        ]),
        join([
            temp_short.as_str(),
            die_limit.as_str(),
            power_short.as_str(),
            "",
            "",
        ]),
        join([temp_short.as_str(), power_short.as_str(), "", "", ""]),
        join([temp_short.as_str(), "", "", "", ""]),
        Some(temp_bare),
    ]
    .into_iter()
    .flatten()
    .collect();
    fit(&candidates, width)
}

/// Pane title. With more than one card the card count is always in it, so a
/// frame too short for the tab strip still says that cards exist.
fn pane_title(cards: &[GpuInfo], sel: usize, t: &GpuTotals, width: usize) -> String {
    let mut candidates: Vec<String> = Vec::new();
    if cards.len() > 1 {
        // `sel` is the tab index and tab 0 is the aggregate, so the selected
        // card's own 1-based number is `sel`.
        let selected = if sel == TAB_ALL {
            None
        } else {
            cards.get(sel - 1).map(short_name)
        };
        if let Some(name) = selected {
            // One card is on show, so name it. Machine-wide totals are left out:
            // they would contradict the bars underneath, which are this card's.
            candidates.push(format!("GPU {name} [{}/{}]", sel, cards.len()));
            candidates.push(format!("GPU [{}/{}]", sel, cards.len()));
        } else {
            candidates.push(format!(
                "GPU ({} cards · {})",
                t.count,
                vram_pair(t.vram_used, t.vram_total, true)
            ));
            candidates.push(format!(
                "GPU ({} cards · {})",
                t.count,
                vram_pair(t.vram_used, t.vram_total, false)
            ));
        }
        // Both cases fall back to the bare count: it is the one fact about a
        // multi-card host that has to survive a narrow terminal.
        candidates.push(format!("GPU ({} cards)", t.count));
    } else if let Some(name) = cards.first().map(short_name) {
        candidates.push(format!("GPU {name}"));
    }
    candidates.push("GPU".into());
    // Two borders, plus a column of breathing room at each end.
    fit(&candidates, width.saturating_sub(4)).unwrap_or_else(|| "GPU".into())
}

fn inner_area(area: Rect) -> Rect {
    Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// Draw a bar filling `row` with its label to the right, choosing the most
/// detailed label that still leaves a bar worth drawing.
fn bar_and_label(
    f: &mut ratatui::Frame<'_>,
    row: Rect,
    ratio: f64,
    color: ratatui::style::Color,
    candidates: &[String],
) {
    if row.width == 0 || row.height == 0 {
        return;
    }
    const MIN_GAUGE_W: u16 = 6;
    let label = fit(
        candidates,
        usize::from(row.width.saturating_sub(MIN_GAUGE_W + 1)),
    );
    let label_w = label.as_ref().map(|l| l.width() as u16).unwrap_or(0);
    let gap = u16::from(label.is_some());
    let gauge_w = row.width.saturating_sub(label_w + gap);

    // The gap gets a slot of its own. Charge it to the gauge and the gauge draws
    // bar glyphs across it; charge it to the label and the label butts against the
    // bar, because it is left-aligned inside a slot wider than the text.
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(gauge_w),
            Constraint::Length(gap),
            Constraint::Min(0),
        ])
        .split(row);

    f.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(color))
            .label(Span::raw(""))
            .ratio(ratio.clamp(0.0, 1.0)),
        cols[0],
    );
    if let Some(label) = label {
        f.render_widget(
            Paragraph::new(Span::raw(label)).style(Style::default().fg(theme::GPU_TEXT)),
            cols[2],
        );
    }
}

pub fn draw_gpu(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>, sel: usize) {
    let cards = gpu_set(m);
    if cards.is_empty() {
        f.render_widget(Block::default().borders(Borders::ALL).title("GPU"), area);
        if area.height > 2 && area.width > 2 {
            f.render_widget(Paragraph::new("No GPUs"), inner_area(area));
        }
        return;
    }

    let multi = cards.len() > 1;
    let totals = aggregate(&cards);
    // With one card there is nothing to select, so tab 0 is that card.
    let show_all = multi && sel == TAB_ALL;
    let card = if show_all || !multi {
        &cards[0]
    } else {
        cards.get(sel - 1).unwrap_or(&cards[0])
    };

    // Title before anything else: it is what says how many cards there are, and
    // it has to survive a frame too short to draw the strip.
    let title = pane_title(&cards, sel, &totals, usize::from(area.width));
    f.render_widget(Block::default().borders(Borders::ALL).title(title), area);
    if area.height <= 2 || area.width <= 2 {
        return;
    }
    let inner = inner_area(area);

    let labels = if multi {
        tab_labels(&cards, inner.width)
    } else {
        Vec::new()
    };
    let sensors = if show_all {
        per_card_line(&cards, usize::from(inner.width))
    } else {
        sensor_line(card, usize::from(inner.width))
    };
    let head_row = if multi { Row::Strip } else { Row::Name };
    let rows = row_plan(inner.height, head_row, sensors.is_some());
    let rects = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); rows.len()])
        .split(inner);

    for (i, kind) in rows.iter().enumerate() {
        let row = rects[i];
        match *kind {
            Row::Strip => {
                let tabs = Tabs::new(
                    labels
                        .iter()
                        .cloned()
                        .map(|l| Line::from(vec![Span::raw(l)])),
                )
                .divider("|")
                .select(sel.min(labels.len().saturating_sub(1)))
                .style(Style::default().fg(theme::GPU_TAB))
                .highlight_style(
                    Style::default()
                        .fg(theme::GPU_TAB_TEXT)
                        .bg(theme::GPU_TAB_BG)
                        .add_modifier(ratatui::style::Modifier::BOLD),
                );
                f.render_widget(tabs, row);
            }
            Row::Name => {
                // Single card: the title carries the short name, this row keeps
                // the full device string readable.
                f.render_widget(
                    Paragraph::new(Span::raw(card.name.as_deref().unwrap_or("GPU")))
                        .style(Style::default().fg(theme::GPU_TEXT)),
                    row,
                );
            }
            Row::Util => {
                let util = if show_all {
                    totals.util_max
                } else {
                    card.utilization.unwrap_or(0.0).clamp(0.0, 100.0)
                };
                let candidates = if show_all {
                    vec![
                        format!("max: {util:.0}%"),
                        format!("util max: {util:.0}%"),
                        format!("{util:.0}%"),
                    ]
                } else {
                    vec![format!("util: {util:.0}%"), format!("{util:.0}%")]
                };
                bar_and_label(f, row, util as f64 / 100.0, theme::GPU_UTIL, &candidates);
            }
            Row::Vram => {
                let (used, total) = if show_all {
                    (totals.vram_used, totals.vram_total)
                } else {
                    (card.mem_used.unwrap_or(0), card.mem_total.unwrap_or(1))
                };
                let ratio = if total > 0 {
                    (used as f64 / total as f64).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let pct = (ratio * 100.0).round();
                // No ``(99%)`` here: the bar next to it *is* that percentage, and
                // the pane is the narrowest one on the screen. The bare number
                // stays as the last resort when even the bytes will not fit.
                let candidates = vec![
                    format!("vram: {}", vram_pair(used, total, true)),
                    format!("vram: {}", vram_pair(used, total, false)),
                    format!("{}%", pct as u64),
                ];
                bar_and_label(f, row, ratio, theme::GPU_VRAM, &candidates);
            }
            Row::Sensors => {
                if let Some(sensors) = &sensors {
                    f.render_widget(
                        Paragraph::new(Span::raw(sensors.clone()))
                            .style(Style::default().fg(theme::GPU_TEXT)),
                        row,
                    );
                }
            }
        }
    }
}

fn vram_pair(used: u64, total: u64, precise: bool) -> String {
    let f = if precise { human_precise } else { human };
    format!("{}/{}", f(used), f(total))
}

/// One-line GPU strip for compact mode: no device name (it is the first thing to
/// lose value when rows are scarce), just utilisation and VRAM on the single
/// content row between the block borders. With several cards it shows the
/// selected one — the aggregate or one card — and the title says which.
pub fn draw_gpu_compact(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>, sel: usize) {
    let cards = gpu_set(m);
    let count = cards.len();
    let title = if count > 1 {
        // `all` rather than a number for the aggregate tab, so a glance says
        // whether the bar is the machine or a single card.
        let which = if sel == TAB_ALL {
            "all".to_string()
        } else {
            sel.to_string()
        };
        format!("GPU ({which}/{count})")
    } else {
        "GPU".to_string()
    };
    f.render_widget(Block::default().borders(Borders::ALL).title(title), area);

    if area.height < 3 || area.width <= 2 {
        return;
    }
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width - 2,
        height: 1,
    };

    let Some(first) = cards.first() else {
        f.render_widget(Paragraph::new("No GPUs"), inner);
        return;
    };

    let multi = count > 1;
    let show_all = multi && sel == TAB_ALL;
    let totals = aggregate(&cards);
    let card = if show_all || !multi {
        first
    } else {
        cards.get(sel - 1).unwrap_or(first)
    };

    let util = if show_all {
        totals.util_max
    } else {
        card.utilization.unwrap_or(0.0).clamp(0.0, 100.0)
    };
    let (used, total) = if show_all {
        (totals.vram_used, totals.vram_total)
    } else {
        (card.mem_used.unwrap_or(0), card.mem_total.unwrap_or(1))
    };
    let mem_ratio = if total > 0 {
        (used as f64 / total as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let util_label = if show_all {
        format!("util max: {util:.0}%")
    } else {
        format!("util: {util:.0}%")
    };
    let mem_label = format!(
        "vram: {}/{} ({}%)",
        human(used),
        human(total),
        (mem_ratio * 100.0).round() as u64
    );
    // The temperature shown next to the numbers must be the temperature of what
    // the numbers describe: the hottest card on the aggregate tab, this card when
    // a card is selected. Grading the glyph against the right card's own limit
    // matters just as much.
    let temp_label = if show_all || !multi {
        totals
            .temp_max
            .map(|t| format!("hot {:.0}°C {}", t, temp_icon(t, totals.temp_slowdown_c)))
    } else {
        card.temp
            .map(|t| format!("temp {:.0}°C {}", t, temp_icon(t, card.temp_slowdown_c)))
    };

    // Bars are sized explicitly rather than left to stretch: an idle bar renders
    // as empty cells, so a full-width one turns into a long blank run between
    // two labels.
    const MIN_GAUGE_W: u16 = 6;
    const MAX_GAUGE_W: u16 = 24;
    let labels_w = util_label.width() as u16 + mem_label.width() as u16 + 4; // one space each side
    let temp_w = temp_label
        .as_ref()
        .map(|t| t.width() as u16 + 2)
        .unwrap_or(0);
    let gauge_w = inner
        .width
        .saturating_sub(labels_w + temp_w)
        .min(2 * MAX_GAUGE_W)
        .div_euclid(2);

    // Too narrow for bars worth drawing: keep the numbers, drop the bars.
    if gauge_w < MIN_GAUGE_W {
        let mut text = format!("{util_label}  {mem_label}");
        if let Some(t) = &temp_label {
            text = format!("{text}  {t}");
        }
        f.render_widget(
            Paragraph::new(Span::raw(text)).style(Style::default().fg(theme::GPU_TEXT)),
            inner,
        );
        return;
    }

    // Each label leads its own bar. Bar-then-label (as the tall pane does) is
    // ambiguous on one line: with an idle bar rendering empty, the next pair's
    // fill ends up flush against the previous pair's text and reads as its own.
    let mut x = inner.x;
    let mut place = |w: u16| {
        let r = Rect {
            x,
            y: inner.y,
            width: w,
            height: 1,
        };
        x = x.saturating_add(w);
        r
    };
    let label = |text: &str| {
        Paragraph::new(Span::raw(format!(" {text} "))).style(Style::default().fg(theme::GPU_TEXT))
    };

    f.render_widget(label(&util_label), place(util_label.width() as u16 + 2));
    f.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(theme::GPU_UTIL))
            .label(Span::raw(""))
            .ratio(util as f64 / 100.0),
        place(gauge_w),
    );
    f.render_widget(label(&mem_label), place(mem_label.width() as u16 + 2));
    f.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(theme::GPU_VRAM))
            .label(Span::raw(""))
            .ratio(mem_ratio),
        place(gauge_w),
    );
    if let Some(t) = &temp_label {
        f.render_widget(label(t), place(temp_w));
    }
}

/// Click a tab label to show that card; scroll the wheel over the pane to cycle.
/// In compact mode there is no strip to click, so a click anywhere cycles — a
/// kiosk gets one obvious interaction rather than a hidden one.
pub fn gpu_handle_mouse(
    sel: &mut usize,
    mouse: MouseEvent,
    area: Rect,
    cards: &[GpuInfo],
    compact: bool,
) {
    let tabs = tab_count(cards);
    if tabs <= 1 {
        return;
    }
    let inside = mouse.column >= area.x
        && mouse.column < area.x.saturating_add(area.width)
        && mouse.row >= area.y
        && mouse.row < area.y.saturating_add(area.height);
    if !inside {
        return;
    }

    match mouse.kind {
        MouseEventKind::ScrollUp => cycle_tab(sel, cards, tabs - 1),
        MouseEventKind::ScrollDown => cycle_tab(sel, cards, 1),
        // Only the press. A click arrives as a press *and* a release, and cycling
        // on both would jump two cards per click in compact mode.
        MouseEventKind::Down(MouseButton::Left) => {
            if compact {
                cycle_tab(sel, cards, 1);
                return;
            }
            // Only the strip is clickable in the tall pane: clicking a bar must
            // not do something surprising.
            let inner = inner_area(area);
            let strip = Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 1,
            };
            if mouse.row != strip.y
                || !row_plan(inner.height, Row::Strip, true).contains(&Row::Strip)
            {
                return;
            }
            let labels = tab_labels(cards, strip.width);
            for (i, r) in tab_rects(strip, &labels).into_iter().enumerate() {
                if mouse.column >= r.x && mouse.column < r.x + r.width {
                    *sel = i;
                    return;
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use socktop_connector::{GpuInfo, Metrics};

    const GIB: u64 = 1024 * 1024 * 1024;

    fn gpu(name: &str) -> GpuInfo {
        GpuInfo {
            name: Some(name.into()),
            vendor: None,
            utilization: Some(42.0),
            mem_used: Some(4_724_464_025),
            mem_total: Some(17_070_817_280),
            temp: None,
            temp_slowdown_c: None,
            temp_mem_max_c: None,
            power_w: None,
            power_limit_w: None,
            fan_pct: None,
            nvml_index: None,
            uuid: None,
        }
    }

    /// The machine this feature was designed for: two 3090s, one carrying a
    /// model, one nearly idle.
    fn two_3090s() -> Vec<GpuInfo> {
        let mut a = gpu("NVIDIA GeForce RTX 3090");
        a.utilization = Some(87.0);
        a.mem_used = Some(4 * GIB);
        a.mem_total = Some(24 * GIB);
        a.temp = Some(71.0);
        a.temp_slowdown_c = Some(93.0);
        a.temp_mem_max_c = Some(96.0);
        a.power_w = Some(285.0);
        a.power_limit_w = Some(350.0);
        a.fan_pct = Some(45);
        a.uuid = Some("GPU-0".into());
        a.nvml_index = Some(0);

        let mut b = gpu("NVIDIA GeForce RTX 3090");
        b.utilization = Some(3.0);
        b.mem_used = Some(GIB);
        b.mem_total = Some(24 * GIB);
        b.temp = Some(68.0);
        b.temp_slowdown_c = Some(93.0);
        b.uuid = Some("GPU-1".into());
        b.nvml_index = Some(1);
        vec![a, b]
    }

    fn metrics(gpus: Option<Vec<GpuInfo>>, extra: Option<Vec<GpuInfo>>) -> Metrics {
        Metrics {
            sampled_at_ms: None,
            cpu_total: 0.0,
            cpu_per_core: vec![],
            mem_total: 1024,
            mem_used: 0,
            mem_free: 0,
            mem_available: 0,
            swap_total: 0,
            swap_used: 0,
            hostname: "t".into(),
            cpu_temp_c: None,
            disks: vec![],
            networks: vec![],
            top_processes: vec![],
            gpus,
            gpus_extra: extra,
            process_count: Some(0),
        }
    }

    /// Rows of the tall pane as laid out by `Layout`: `GAUGE_H` is 3 and the GPU
    /// pane spans memory + swap, so 6 rows including borders — 4 usable.
    /// Everything the multi-card design claims to fit has to fit in those 4.
    const TALL: u16 = 6;

    fn render_tall(width: u16, height: u16, m: &Metrics, sel: usize) -> String {
        dump(&tall_buf(width, height, m, sel))
    }

    fn tall_buf(width: u16, height: u16, m: &Metrics, sel: usize) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                draw_gpu(f, Rect::new(0, 0, area.width, area.height), Some(m), sel);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn render(width: u16, m: &Metrics, sel: usize) -> String {
        dump(&compact_buf(width, m, sel))
    }

    fn compact_buf(width: u16, m: &Metrics, sel: usize) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, 3)).unwrap();
        terminal
            .draw(|f| draw_gpu_compact(f, Rect::new(0, 0, width, 3), Some(m), sel))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn dump(buf: &ratatui::buffer::Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area().height {
            for x in 0..buf.area().width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// Which tab the strip is highlighting, found by the selected-tab background
    /// colour. Symbols alone cannot show it, and a strip that highlights the wrong
    /// card is worse than no strip.
    fn highlighted_tab(
        buf: &ratatui::buffer::Buffer,
        labels: &[String],
        strip: Rect,
    ) -> Option<usize> {
        let rects = tab_rects(strip, labels);
        (0..buf.area().width).find_map(|x| {
            if buf[(x, strip.y)].style().bg != Some(theme::GPU_TAB_BG) {
                return None;
            }
            rects.iter().position(|r| x >= r.x && x < r.x + r.width)
        })
    }

    // ---- wire compatibility ------------------------------------------------

    /// An agent that has never heard of `gpus_extra` keeps the pane exactly as it
    /// was: one card, no tab strip, no card count in the title.
    #[test]
    fn old_agent_payload_draws_a_single_card() {
        let m = metrics(Some(vec![gpu("NVIDIA GeForce RTX 3080 Ti")]), None);
        let out = render_tall(60, TALL, &m, 0);
        assert!(!out.contains("All"), "tab strip for one card:\n{out}");
        assert!(!out.contains("cards"), "card count for one card:\n{out}");
        assert!(out.contains("NVIDIA GeForce RTX 3080 Ti"), "{out}");
        assert!(out.contains("util: 42%"), "{out}");
        assert!(out.contains("vram: 4.40GB/15.90GB"), "{out}");
        assert!(
            !out.contains("(28%)"),
            "the bar already shows that percentage:\n{out}"
        );
    }

    /// The bar and its label must not touch. A bar that is full is the only way
    /// to see the join, since an unfilled gauge cell is blank either way.
    #[test]
    fn a_full_bar_leaves_exactly_one_column_before_its_label() {
        let both = two_3090s();
        let mut full = both[0].clone();
        full.mem_used = full.mem_total;
        let m = metrics(Some(vec![full]), Some(vec![both[1].clone()]));
        let out = render_tall(44, TALL, &m, 1);
        let line = out
            .lines()
            .find(|l| l.contains("vram:"))
            .unwrap_or_else(|| panic!("no vram row in\n{out}"));
        let at = line.find("vram:").unwrap();
        assert!(line[..at].ends_with(' '), "label touches the bar:\n{out}");
        assert!(
            line[..at].ends_with("█ "),
            "the bar does not reach the gap, or wastes more than one column:\n{out}"
        );
        // The freed characters went to the bar, not to a blank right margin.
        assert!(
            line.ends_with("vram: 24.00GB/24.00GB│"),
            "the label no longer ends at the border:\n{out}"
        );
    }

    /// Both cards arrive, and the primary keeps its place at the front.
    #[test]
    fn primary_and_extra_recombine_in_order() {
        let both = two_3090s();
        let m = metrics(Some(vec![both[0].clone()]), Some(vec![both[1].clone()]));
        let set = gpu_set(Some(&m));
        assert_eq!(set.len(), 2);
        assert_eq!(set[0].uuid.as_deref(), Some("GPU-0"));
        assert_eq!(set[1].uuid.as_deref(), Some("GPU-1"));
    }

    /// An agent that reports its primary card through the display driver *and*
    /// again through NVML must not list it twice.
    #[test]
    fn the_same_card_is_never_listed_twice() {
        let both = two_3090s();
        let m = metrics(Some(vec![both[0].clone()]), Some(both.clone()));
        assert_eq!(gpu_set(Some(&m)).len(), 2);

        // Same physical card, but only one side carries a UUID: the weak key has
        // to catch it.
        let mut unlabelled = both[0].clone();
        unlabelled.uuid = None;
        unlabelled.nvml_index = None;
        let m = metrics(Some(vec![unlabelled]), Some(vec![both[0].clone()]));
        assert_eq!(gpu_set(Some(&m)).len(), 1);
    }

    /// Two cards that differ only in size are different cards.
    #[test]
    fn same_name_different_card_stays_two_cards() {
        let mut big = gpu("NVIDIA GeForce RTX 3090");
        big.mem_total = Some(24 * GIB);
        let mut bigger = gpu("NVIDIA GeForce RTX 3090");
        bigger.mem_total = Some(48 * GIB);
        let m = metrics(Some(vec![big]), Some(vec![bigger]));
        assert_eq!(gpu_set(Some(&m)).len(), 2);
    }

    // ---- aggregation ------------------------------------------------------

    /// The reason the aggregate bar is a maximum: averaging two duty cycles turns
    /// "one card pegged, one idle" into "both cards half used".
    #[test]
    fn aggregate_utilisation_is_the_max_not_the_mean() {
        let t = aggregate(&two_3090s());
        assert_eq!(t.util_max, 87.0);
        assert_ne!(t.util_max, (87.0 + 3.0) / 2.0);
    }

    #[test]
    fn aggregate_vram_and_power_are_sums() {
        let t = aggregate(&two_3090s());
        assert_eq!(t.vram_used, 5 * GIB);
        assert_eq!(t.vram_total, 48 * GIB);
        assert_eq!(t.power_w, 285.0);
        assert_eq!(t.count, 2);
    }

    /// Hottest card for the headline, tightest limit for the judgement: together
    /// they answer "is any card about to throttle".
    #[test]
    fn temps_aggregate_to_hottest_and_tightest_limit() {
        let mut cards = two_3090s();
        cards[1].temp = Some(95.0);
        cards[1].temp_slowdown_c = Some(88.0);
        let t = aggregate(&cards);
        assert_eq!(t.temp_max, Some(95.0));
        assert_eq!(t.temp_slowdown_c, Some(88.0));
    }

    /// The bug this prevents: a card with no temperature reading counted as 0°C,
    /// dragging the machine-wide "hottest" down to nothing.
    #[test]
    fn a_card_without_sensors_aggregates_to_none_not_zero() {
        let t = aggregate(&[gpu("GPU A"), gpu("GPU B")]);
        assert_eq!(t.temp_max, None);
        assert_eq!(t.temp_slowdown_c, None);
        assert_eq!(t.power_w, 0.0);
    }

    #[test]
    fn temp_glyph_grades_against_the_cards_own_limit() {
        // Same 90°C read on two cards, opposite verdicts.
        assert_eq!(temp_icon(90.0, Some(93.0)), "🔥");
        assert_eq!(temp_icon(90.0, Some(105.0)), "⚠️");
        // A 3090 working at 71°C is not a warning.
        assert_eq!(temp_icon(71.0, Some(93.0)), "😎");
    }

    // ---- selection --------------------------------------------------------

    #[test]
    fn one_card_has_no_aggregate_tab() {
        assert_eq!(tab_count(&[gpu("a")]), 1);
        assert_eq!(tab_count(&two_3090s()), 3); // All + 2 cards
        assert_eq!(tab_labels(&[gpu("a")], 60), Vec::<String>::new());
    }

    /// Cards can vanish between refreshes (driver reload, agent restart), so a
    /// stale selection has to move rather than index past the end.
    #[test]
    fn selection_clamps_when_cards_disappear() {
        let mut sel = 5;
        clamp_tab(&mut sel, &two_3090s());
        assert_eq!(sel, 2);
        clamp_tab(&mut sel, &[gpu("a")]);
        assert_eq!(sel, 0);
    }

    #[test]
    fn cycling_wraps_over_the_aggregate_tab() {
        let cards = two_3090s();
        let mut sel = 0;
        cycle_tab(&mut sel, &cards, 1);
        assert_eq!(sel, 1);
        cycle_tab(&mut sel, &cards, tab_count(&cards) - 1);
        assert_eq!(sel, 0);
    }

    /// The strip has to highlight the tab that is actually selected: a strip that
    /// lights up the wrong card tells the user the wrong thing about which numbers
    /// are on screen.
    #[test]
    fn the_highlighted_tab_follows_the_selection() {
        let both = two_3090s();
        let m = metrics(Some(vec![both[0].clone()]), Some(vec![both[1].clone()]));
        let strip = Rect {
            x: 1,
            y: 1,
            width: 58,
            height: 1,
        };
        let labels = tab_labels(&both, strip.width);
        for sel in 0..tab_count(&both) {
            let buf = tall_buf(60, TALL, &m, sel);
            assert_eq!(
                highlighted_tab(&buf, &labels, strip),
                Some(sel),
                "selection {sel} not highlighted:\n{}",
                dump(&buf)
            );
        }
    }

    // ---- layout -----------------------------------------------------------

    /// Four usable rows is the whole budget the real layout gives this pane, and
    /// All + two cards has to fit inside it.
    #[test]
    fn two_cards_fit_their_strip_in_the_real_pane_height() {
        let both = two_3090s();
        let m = metrics(Some(vec![both[0].clone()]), Some(vec![both[1].clone()]));
        let out = render_tall(60, TALL, &m, 0);
        assert!(out.contains("All"), "{out}");
        assert!(out.contains("0:RTX 3090"), "{out}");
        assert!(out.contains("1:RTX 3090"), "{out}");
        // Aggregate view: max utilisation, summed VRAM.
        assert!(out.contains("max: 87%"), "{out}");
        assert!(out.contains("5.00GB/48.00GB"), "{out}");
        // And both cards named with their own numbers underneath.
        assert!(out.contains("0 87%"), "{out}");
        assert!(out.contains("1 3%"), "{out}");
    }

    /// Selecting a tab switches that card in, sensors and all.
    #[test]
    fn selecting_a_tab_switches_to_that_card() {
        let both = two_3090s();
        let m = metrics(Some(vec![both[0].clone()]), Some(vec![both[1].clone()]));
        let out = render_tall(60, TALL, &m, 2);
        assert!(out.contains("GPU RTX 3090 [2/2]"), "{out}");
        assert!(out.contains("util: 3%"), "{out}");
        assert!(out.contains("1.00GB/24.00GB"), "{out}");
        assert!(out.contains("temp: 68.0°C"), "{out}");
        assert!(
            !out.contains("max:"),
            "aggregate shown for one card:\n{out}"
        );
    }

    /// A pane too short for the strip must still say there is more than one card;
    /// a silent single-card view is indistinguishable from a dropped card.
    #[test]
    fn a_short_pane_still_reports_the_card_count() {
        let both = two_3090s();
        let m = metrics(Some(vec![both[0].clone()]), Some(vec![both[1].clone()]));
        let out = render_tall(60, 4, &m, 0);
        assert!(!out.contains("All"), "strip survived a 2-row pane:\n{out}");
        assert!(out.contains("2 cards"), "{out}");
        assert!(out.contains("87%"), "{out}");
    }

    /// What gives up when rows are scarce: sensors, then the strip — never a bar.
    #[test]
    fn sensors_are_dropped_before_the_bars_and_the_strip() {
        let both = two_3090s();
        let m = metrics(Some(vec![both[0].clone()]), Some(vec![both[1].clone()]));
        let out = render_tall(60, 5, &m, 2); // 3 usable rows
        assert!(out.contains("All"), "{out}");
        assert!(out.contains("util: 3%"), "{out}");
        assert!(out.contains("vram:"), "{out}");
        assert!(!out.contains("temp:"), "sensors outlived the strip:\n{out}");

        // Two usable rows: the bars survive on their own.
        let out = render_tall(60, 4, &m, 2);
        assert!(out.contains("util: 3%"), "{out}");
        assert!(out.contains("vram:"), "{out}");
    }

    /// Every card keeps a label down to the narrowest pane that can hold a strip;
    /// shedding goes to bare indexes before it ever drops a card.
    #[test]
    fn tab_labels_shed_detail_but_never_a_card() {
        let cards = two_3090s();
        assert_eq!(tab_labels(&cards, 60), ["All", "0:RTX 3090", "1:RTX 3090"]);

        for width in [12u16, 14, 16, 18, 20, 24, 30, 40] {
            let labels = tab_labels(&cards, width);
            assert_eq!(labels.len(), 3, "width {width}: {labels:?}");
            assert_eq!(labels[0], "All", "width {width}");
        }
        assert_eq!(tab_labels(&cards, 40), ["All", "0:RTX 3090", "1:RTX 3090"]);
        assert_eq!(tab_labels(&cards, 24), ["All", "0:3090", "1:3090"]);
        assert_eq!(tab_labels(&cards, 20), ["All", "0", "1"]);
        assert_eq!(tab_labels(&cards, 8), ["All", "0", "1"]);
    }

    #[test]
    fn the_aggregate_title_carries_totals_then_the_count() {
        let cards = two_3090s();
        let t = aggregate(&cards);
        assert_eq!(
            pane_title(&cards, 0, &t, 60),
            "GPU (2 cards · 5.00GB/48.00GB)"
        );
        assert_eq!(
            pane_title(&cards, 0, &t, 40),
            "GPU (2 cards · 5.00GB/48.00GB)"
        );
        assert_eq!(pane_title(&cards, 0, &t, 20), "GPU (2 cards)");
        assert_eq!(pane_title(&cards, 0, &t, 6), "GPU");
    }

    /// A selected card is named in the title, rather than machine-wide totals
    /// sitting above bars that belong to one card.
    #[test]
    fn a_selected_card_is_named_in_the_title() {
        let cards = two_3090s();
        let t = aggregate(&cards);
        assert_eq!(pane_title(&cards, 2, &t, 60), "GPU RTX 3090 [2/2]");
        assert_eq!(pane_title(&cards, 1, &t, 22), "GPU RTX 3090 [1/2]");
        assert_eq!(pane_title(&cards, 1, &t, 14), "GPU [1/2]");
        assert_eq!(pane_title(&cards, 1, &t, 10), "GPU");
        assert_eq!(pane_title(&cards, 1, &t, 6), "GPU");
    }

    #[test]
    fn short_names_drop_the_marketing_prefix() {
        assert_eq!(short_name(&gpu("NVIDIA GeForce RTX 3090")), "RTX 3090");
        assert_eq!(short_name(&gpu("AMD Radeon RX 7900 XTX")), "RX 7900 XTX");
        assert_eq!(short_name(&gpu("Intel")), "Intel");
        assert_eq!(short_name(&gpu("")), "GPU");
    }

    // ---- sensor line ------------------------------------------------------

    #[test]
    fn sensor_line_prefers_the_richest_form_that_fits() {
        let both = two_3090s();
        let wide = sensor_line(&both[0], 120).expect("sensors");
        assert!(wide.contains("temp: 71.0°C"), "{wide}");
        assert!(wide.contains("vram max 96°C"), "{wide}");
        assert!(wide.contains("power: 285/350W"), "{wide}");
        assert!(wide.contains("fan 45%"), "{wide}");

        // A mid-width pane keeps the temperature's glyph and the limit that glyph
        // is graded against, and gives up the labels instead.
        let mid = sensor_line(&both[0], 34).expect("sensors");
        assert!(mid.width() <= 34, "{mid}");
        assert!(mid.contains("71°C"), "{mid}");
        assert!(mid.contains("limit 93°C"), "die limit dropped: {mid}");
        assert!(
            mid.contains('😎') || mid.contains('⚠') || mid.contains('🔥'),
            "glyph dropped while the number stayed: {mid}"
        );

        let tight = sensor_line(&both[0], 20).expect("sensors");
        assert!(tight.width() <= 20, "{tight}");
        assert!(
            tight.contains("°C"),
            "temperature survived everything but: {tight}"
        );

        assert_eq!(sensor_line(&both[0], 6).as_deref(), Some("71°C"));
    }

    /// No sensors reported at all means no row, not a row of dashes.
    #[test]
    fn no_sensors_means_no_sensor_row() {
        let m = metrics(Some(vec![gpu("a"), gpu("b")]), None);
        let out = render_tall(60, TALL, &m, 1);
        assert!(!out.contains("temp"), "{out}");
        assert!(out.contains("util: 42%"), "{out}");
    }

    /// A stopped or passive fan is an error from NVML rather than 0%, so it stays
    /// out of the label instead of claiming a dead fan.
    #[test]
    fn an_absent_fan_is_not_reported_as_zero() {
        let mut no_fan = two_3090s()[0].clone();
        no_fan.fan_pct = None;
        let line = sensor_line(&no_fan, 120).expect("sensors");
        assert!(!line.contains("fan"), "{line}");

        // A fan the driver really does report at 0% is shown as 0%.
        let mut stopped = no_fan.clone();
        stopped.fan_pct = Some(0);
        assert!(sensor_line(&stopped, 120).unwrap().contains("fan 0%"));
    }

    // ---- compact strip (kiosk) -------------------------------------------

    /// Compact mode drops the device name — the row is one line and the numbers
    /// are what the space is for.
    #[test]
    fn compact_strip_omits_the_device_name() {
        let m = metrics(Some(vec![gpu("NVIDIA GeForce RTX 5080")]), None);
        let out = render(80, &m, 0);
        assert!(!out.contains("NVIDIA"), "name leaked into strip:\n{out}");
        assert!(out.contains("util: 42%"), "{out}");
        assert!(out.contains("vram: 4.4GB/15.9GB (28%)"), "{out}");
    }

    /// Several cards cannot fit on one line, so the title says which one is shown
    /// rather than silently showing only the first.
    #[test]
    fn multiple_gpus_are_flagged_in_the_title() {
        let one = render(80, &metrics(Some(vec![gpu("a")]), None), 0);
        assert!(one.contains("GPU") && !one.contains("1/"), "{one}");

        let both = two_3090s();
        let m = metrics(Some(vec![both[0].clone()]), Some(vec![both[1].clone()]));
        let all = render(80, &m, 0);
        assert!(all.contains("GPU (all/2)"), "{all}");
        assert!(all.contains("util max: 87%"), "{all}");
        let second = render(80, &m, 2);
        assert!(second.contains("GPU (2/2)"), "{second}");
        assert!(second.contains("util: 3%"), "{second}");
    }

    /// The hottest card belongs on the aggregate strip: without it a multi-card
    /// box looks cooler than it is.
    #[test]
    fn compact_aggregate_shows_the_hottest_card() {
        let both = two_3090s();
        let m = metrics(Some(vec![both[0].clone()]), Some(vec![both[1].clone()]));
        let out = render(100, &m, 0);
        assert!(out.contains("hot 71°C"), "{out}");
    }

    /// Narrow terminals drop the gauges rather than rendering two-cell stubs, but
    /// must never drop the numbers.
    #[test]
    fn narrow_strip_keeps_the_numbers() {
        let m = metrics(Some(vec![gpu("a")]), None);
        for width in [20u16, 30, 40, 47, 48, 80, 200] {
            let out = render(width, &m, 0);
            if width >= 40 {
                assert!(out.contains("util: 42%"), "width {width}:\n{out}");
            }
            // No panic, and the block always closes on the last row.
            assert_eq!(out.lines().count(), 3, "width {width}");
        }
    }

    #[test]
    fn missing_gpu_payload_does_not_panic() {
        assert!(render(80, &metrics(None, None), 0).contains("No GPUs"));
        assert!(render(80, &metrics(Some(vec![]), None), 0).contains("No GPUs"));
        assert!(
            render_tall(60, TALL, &metrics(None, None), 0).contains("No GPUs"),
            "tall pane"
        );
    }

    /// Selection state from an earlier frame must never index out of range.
    #[test]
    fn stale_selections_are_tolerated() {
        let both = two_3090s();
        let m = metrics(Some(vec![both[0].clone()]), Some(vec![both[1].clone()]));
        for sel in [0usize, 1, 2, 3, 99] {
            let _ = render_tall(60, TALL, &m, sel);
            let _ = render(80, &m, sel);
        }
    }

    // ---- mouse ------------------------------------------------------------

    fn click(col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }
    }

    fn wheel(col: u16, row: u16, down: bool) -> MouseEvent {
        MouseEvent {
            kind: if down {
                MouseEventKind::ScrollDown
            } else {
                MouseEventKind::ScrollUp
            },
            column: col,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }
    }

    /// Clicking a tab label selects that card — and the rectangles have to match
    /// what `Tabs` actually painted, or the wrong card gets picked.
    #[test]
    fn clicking_a_tab_label_selects_that_card() {
        let cards = two_3090s();
        let area = Rect::new(0, 0, 60, TALL);
        let strip = Rect {
            x: 1,
            y: 1,
            width: 58,
            height: 1,
        };
        let labels = tab_labels(&cards, strip.width);
        let rects = tab_rects(strip, &labels);
        assert_eq!(rects.len(), 3);

        for (i, r) in rects.iter().enumerate() {
            let mut sel = 0;
            gpu_handle_mouse(&mut sel, click(r.x, 1), area, &cards, false);
            assert_eq!(sel, i, "left edge of tab {i} at column {}", r.x);
            let mut sel_last = 0;
            gpu_handle_mouse(
                &mut sel_last,
                click(r.x + r.width - 1, 1),
                area,
                &cards,
                false,
            );
            assert_eq!(sel_last, i, "right edge of tab {i}");
        }
    }

    /// Clicking a bar is not a selection: it should do nothing surprising.
    #[test]
    fn clicking_a_bar_changes_nothing() {
        let cards = two_3090s();
        let mut sel = 0;
        gpu_handle_mouse(
            &mut sel,
            click(3, 3),
            Rect::new(0, 0, 60, TALL),
            &cards,
            false,
        );
        assert_eq!(sel, 0);
    }

    /// A click reaches the app as a press *and* a release. Cycling on both would
    /// skip a card in compact mode, where a click is the only pointer control.
    #[test]
    fn one_click_cycles_once() {
        let cards = two_3090s();
        let mut sel = TAB_ALL;
        let area = Rect::new(0, 0, 60, 3);
        let mut release = click(30, 1);
        release.kind = MouseEventKind::Up(MouseButton::Left);
        gpu_handle_mouse(&mut sel, click(30, 1), area, &cards, true);
        gpu_handle_mouse(&mut sel, release, area, &cards, true);
        assert_eq!(
            sel, 1,
            "press plus release must land on the next card, not the one after"
        );
    }

    /// The temperature in the compact row must describe what the rest of the row
    /// describes: the hottest card on the aggregate, the selected card otherwise.
    #[test]
    fn compact_temperature_follows_the_selection() {
        let m = metrics(Some(two_3090s()), None);
        let all = render(80, &m, TAB_ALL);
        assert!(all.contains("hot 71°C"), "{all}");

        let second = render(80, &m, 2);
        assert!(second.contains("temp 68°C"), "{second}");
        assert!(
            !second.contains("71°C"),
            "aggregate temperature shown while one card is selected:\n{second}"
        );
    }

    #[test]
    fn wheel_scrolling_cycles_cards() {
        let cards = two_3090s();
        let area = Rect::new(0, 0, 60, TALL);
        let mut sel = 0;
        gpu_handle_mouse(&mut sel, wheel(5, 3, true), area, &cards, false);
        assert_eq!(sel, 1);
        gpu_handle_mouse(&mut sel, wheel(5, 3, false), area, &cards, false);
        assert_eq!(sel, 0);
    }

    /// Kiosk compact mode: one obvious gesture, tap anywhere to cycle.
    #[test]
    fn compact_click_cycles_without_a_strip() {
        let cards = two_3090s();
        let area = Rect::new(0, 0, 80, 3);
        let mut sel = 0;
        for expected in [1, 2, 0] {
            gpu_handle_mouse(&mut sel, click(40, 1), area, &cards, true);
            assert_eq!(sel, expected);
        }
    }

    /// Outside this pane, clicks belong to whatever pane they really are in.
    #[test]
    fn clicks_outside_the_pane_are_ignored() {
        let cards = two_3090s();
        let mut sel = 0;
        gpu_handle_mouse(
            &mut sel,
            click(70, 1),
            Rect::new(0, 0, 60, TALL),
            &cards,
            true,
        );
        assert_eq!(sel, 0);
    }

    /// A pane with one card has nothing to switch, so no gesture claims to.
    #[test]
    fn one_card_ignores_every_switch_gesture() {
        let cards = [gpu("a")];
        let area = Rect::new(0, 0, 60, TALL);
        let mut sel = 0;
        gpu_handle_mouse(&mut sel, click(1, 1), area, &cards, false);
        gpu_handle_mouse(&mut sel, wheel(5, 2, true), area, &cards, false);
        gpu_handle_mouse(&mut sel, click(40, 1), area, &cards, true);
        assert_eq!(sel, 0);
    }
}
