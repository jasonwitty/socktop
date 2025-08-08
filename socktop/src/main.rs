use std::{collections::VecDeque, env, error::Error, io, time::{Duration, Instant}};

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::{SinkExt, StreamExt};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Gauge, Row, Sparkline, Table, Cell},
    Terminal,
    text::{Line, Span},
};

use ratatui::style::{Modifier}; 

use serde::Deserialize;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};


#[derive(Debug, Deserialize, Clone)]
struct Disk { name: String, total: u64, available: u64 }
#[derive(Debug, Deserialize, Clone)]
struct Network { received: u64, transmitted: u64 }
#[derive(Debug, Deserialize, Clone)]
struct ProcessInfo {
    pid: i32,
    name: String,
    cpu_usage: f32,
    mem_bytes: u64,
}

#[derive(Debug, Deserialize, Clone)]
struct Metrics {
    cpu_total: f32,
    cpu_per_core: Vec<f32>,
    mem_total: u64,
    mem_used: u64,
    swap_total: u64,
    swap_used: u64,
    process_count: usize,
    hostname: String,
    cpu_temp_c: Option<f32>,
    disks: Vec<Disk>,
    networks: Vec<Network>,
    top_processes: Vec<ProcessInfo>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} ws://HOST:PORT/ws", args[0]);
        std::process::exit(1);
    }
    let url = &args[1];
    let (mut ws, _) = connect_async(url).await?;

    // Terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // State
    let mut last_metrics: Option<Metrics> = None;
    let mut cpu_hist: VecDeque<u64> = VecDeque::with_capacity(600);

    let mut per_core_hist: Vec<VecDeque<u16>> = Vec::new(); // one deque per core
    const CORE_HISTORY: usize = 60; // ~30s if you tick every 500ms

    // Network: keep totals across ALL ifaces + timestamp
    let mut last_net_totals: Option<(u64, u64, Instant)> = None;
    let mut rx_hist: VecDeque<u64> = VecDeque::with_capacity(600);
    let mut tx_hist: VecDeque<u64> = VecDeque::with_capacity(600);
    let mut rx_peak: u64 = 0;
    let mut tx_peak: u64 = 0;

    let mut should_quit = false;

    loop {
        while event::poll(Duration::from_millis(10))? {
            if let Event::Key(k) = event::read()? {
                if matches!(k.code, KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc) {
                    should_quit = true;
                }
            }
        }
        if should_quit { break; }

        ws.send(Message::Text("get_metrics".into())).await.ok();

        if let Some(Ok(Message::Text(json))) = ws.next().await {
            if let Ok(m) = serde_json::from_str::<Metrics>(&json) {
                // CPU history
                let v = m.cpu_total.clamp(0.0, 100.0).round() as u64;
                push_capped(&mut cpu_hist, v, 600);

                // NET: sum across all ifaces, compute KB/s via elapsed time
                let now = Instant::now();
                let rx_total = m.networks.iter().map(|n| n.received).sum::<u64>();
                let tx_total = m.networks.iter().map(|n| n.transmitted).sum::<u64>();
                let (rx_kb, tx_kb) = if let Some((prx, ptx, pts)) = last_net_totals {
                    let dt = now.duration_since(pts).as_secs_f64().max(1e-6);
                    let rx = ((rx_total.saturating_sub(prx)) as f64 / dt / 1024.0).round() as u64;
                    let tx = ((tx_total.saturating_sub(ptx)) as f64 / dt / 1024.0).round() as u64;
                    (rx, tx)
                } else { (0, 0) };
                last_net_totals = Some((rx_total, tx_total, now));
                push_capped(&mut rx_hist, rx_kb, 600);
                push_capped(&mut tx_hist, tx_kb, 600);
                rx_peak = rx_peak.max(rx_kb);
                tx_peak = tx_peak.max(tx_kb);

                if let Some(m) = last_metrics.as_ref() {
                // resize history buffers if core count changes
                    if per_core_hist.len() != m.cpu_per_core.len() {
                        per_core_hist = (0..m.cpu_per_core.len())
                            .map(|_| VecDeque::with_capacity(CORE_HISTORY))
                            .collect();
                    }
                }
                // push latest per-core samples
                if let Some(m) = last_metrics.as_ref() {
                    for (i, v) in m.cpu_per_core.iter().enumerate() {
                        let v = v.clamp(0.0, 100.0).round() as u16;
                        push_capped(&mut per_core_hist[i], v, CORE_HISTORY);
                    }
             }

                last_metrics = Some(m);
            }
        }

        terminal.draw(|f| {
            let area = f.area();

            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Ratio(1, 3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Min(10),
                ])
                .split(area);

            draw_header(f, rows[0], last_metrics.as_ref());

            let top = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
                .split(rows[1]);

            draw_cpu_avg_graph(f, top[0], &cpu_hist, last_metrics.as_ref());
            draw_per_core_bars(f, top[1], last_metrics.as_ref(), &per_core_hist);

            draw_mem(f, rows[2], last_metrics.as_ref());
            draw_swap(f, rows[3], last_metrics.as_ref());

            let bottom = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
                .split(rows[4]);

            let left_stack = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(6), Constraint::Length(4), Constraint::Length(4)])
                .split(bottom[0]);

            draw_disks(f, left_stack[0], last_metrics.as_ref());
            draw_net_spark(
                f,
                left_stack[1],
                &format!("Download (KB/s) — now: {} | peak: {}", rx_hist.back().copied().unwrap_or(0), rx_peak),
                &rx_hist,
                Color::Green,
            );
            draw_net_spark(
                f,
                left_stack[2],
                &format!("Upload (KB/s) — now: {} | peak: {}", tx_hist.back().copied().unwrap_or(0), tx_peak),
                &tx_hist,
                Color::Blue,
            );

            draw_top_processes(f, bottom[1], last_metrics.as_ref());
        })?;

        sleep(Duration::from_millis(500)).await;
    }

    disable_raw_mode()?;
    let backend = terminal.backend_mut();
    execute!(backend, LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn push_capped<T>(dq: &mut VecDeque<T>, v: T, cap: usize) {
    if dq.len() == cap { dq.pop_front(); }
    dq.push_back(v);
}

fn human(b: u64) -> String {
    const K: f64 = 1024.0;
    let b = b as f64;
    if b < K { return format!("{b:.0}B"); }
    let kb = b / K;
    if kb < K { return format!("{kb:.1}KB"); }
    let mb = kb / K;
    if mb < K { return format!("{mb:.1}MB"); }
    let gb = mb / K;
    if gb < K { return format!("{gb:.1}GB"); }
    let tb = gb / K;
    format!("{tb:.2}TB")
}

fn draw_header(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    let title = if let Some(mm) = m {
        let temp = mm.cpu_temp_c.map(|t| {
            let icon = if t < 50.0 { "😎" } else if t < 85.0 { "⚠️" } else { "🔥" };
            format!("CPU Temp: {:.1}°C {}", t, icon)
        }).unwrap_or_else(|| "CPU Temp: N/A".into());
        format!("socktop — host: {} | {}  (press 'q' to quit)", mm.hostname, temp)
    } else {
        "socktop — connecting... (press 'q' to quit)".into()
    };
    f.render_widget(Block::default().title(title).borders(Borders::BOTTOM), area);
}

fn draw_cpu_avg_graph(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    hist: &VecDeque<u64>,
    m: Option<&Metrics>,
) {
    let title = if let Some(mm) = m { format!("CPU avg (now: {:>5.1}%)", mm.cpu_total) } else { "CPU avg".into() };
    let max_points = area.width.saturating_sub(2) as usize;
    let start = hist.len().saturating_sub(max_points);
    let data: Vec<u64> = hist.iter().skip(start).cloned().collect();
    let spark = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title(title))
        .data(&data)
        .max(100)
        .style(Style::default().fg(Color::Cyan));
    f.render_widget(spark, area);
}

fn draw_per_core_bars(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    m: Option<&Metrics>,
    // 👇 add this param
    per_core_hist: &Vec<VecDeque<u16>>,
) {
    // frame
    f.render_widget(Block::default().borders(Borders::ALL).title("Per-core"), area);
    let Some(mm) = m else { return; };

    let inner = Rect { x: area.x + 1, y: area.y + 1, width: area.width.saturating_sub(2), height: area.height.saturating_sub(2) };
    if inner.height == 0 { return; }

    // one row per core
    let rows = inner.height as usize;
    let show_n = rows.min(mm.cpu_per_core.len());
    let constraints: Vec<Constraint> = (0..show_n).map(|_| Constraint::Length(1)).collect();
    let vchunks = Layout::default().direction(Direction::Vertical).constraints(constraints).split(inner);

    for i in 0..show_n {
        let rect = vchunks[i];

        // split each row: sparkline (history) | stat text
        let hchunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(6), Constraint::Length(12)]) // was 10 → now 12
            .split(rect);

        let curr = mm.cpu_per_core[i].clamp(0.0, 100.0);
        let older = per_core_hist.get(i)
            .and_then(|d| d.iter().rev().nth(20).copied()) // ~10s back
            .map(|v| v as f32)
            .unwrap_or(curr);
        let trend = if curr > older + 0.2 { "↑" } 
                    else if curr + 0.2 < older { "↓" } 
                    else { "╌" };

        // colors by current load
        let fg = match curr {
            x if x < 25.0 => Color::Green,
            x if x < 60.0 => Color::Yellow,
            _ => Color::Red,
        };

        // history
        let hist: Vec<u64> = per_core_hist
            .get(i)
            .map(|d| {
                let max_points = hchunks[0].width as usize;
                let start = d.len().saturating_sub(max_points);
                d.iter().skip(start).map(|&v| v as u64).collect()
            })
            .unwrap_or_default();

        // sparkline
        let spark = Sparkline::default()
            .data(&hist)
            .max(100)
            .style(Style::default().fg(fg));
        f.render_widget(spark, hchunks[0]); // ✅ render_widget on rect

        // right stat “cpuN  37.2%  ↑”
        let label = format!("cpu{:<2}{}{:>5.1}%", i, trend, curr);
        let line = Line::from(Span::styled(label, Style::default().fg(fg).add_modifier(Modifier::BOLD)));
        let block = Block::default(); // no borders per row to keep it clean
        f.render_widget(block, hchunks[1]);
        f.render_widget(ratatui::widgets::Paragraph::new(line).right_aligned(), hchunks[1]);
    }
}

fn draw_mem(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    let (used, total, pct) = if let Some(mm) = m {
        let pct = if mm.mem_total > 0 { (mm.mem_used as f64 / mm.mem_total as f64 * 100.0) as u16 } else { 0 };
        (mm.mem_used, mm.mem_total, pct)
    } else { (0, 0, 0) };

    let g = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Memory"))
        .gauge_style(Style::default().fg(Color::Magenta))
        .percent(pct)
        .label(format!("{} / {}", human(used), human(total)));
    f.render_widget(g, area);
}

fn draw_swap(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    let (used, total, pct) = if let Some(mm) = m {
        let pct = if mm.swap_total > 0 { (mm.swap_used as f64 / mm.swap_total as f64 * 100.0) as u16 } else { 0 };
        (mm.swap_used, mm.swap_total, pct)
    } else { (0, 0, 0) };

    let g = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Swap"))
        .gauge_style(Style::default().fg(Color::Yellow))
        .percent(pct)
        .label(format!("{} / {}", human(used), human(total)));
    f.render_widget(g, area);
}

fn draw_disks(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    // Panel frame
    f.render_widget(Block::default().borders(Borders::ALL).title("Disks"), area);

    let Some(mm) = m else { return; };

    // Inner area inside the "Disks" panel
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.height < 3 { return; }

    // Each disk gets a 3-row card: [title line] + [gauge line] + [spacer]
    // If we run out of height, we show as many as we can.
    let per_disk_h = 3u16;
    let max_cards = (inner.height / per_disk_h).min(mm.disks.len() as u16) as usize;

    // Build rows layout (Length(3) per disk)
    let constraints: Vec<Constraint> = (0..max_cards).map(|_| Constraint::Length(per_disk_h)).collect();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    for (i, slot) in rows.iter().enumerate() {
        let d = &mm.disks[i];
        let used = d.total.saturating_sub(d.available);
        let ratio = if d.total > 0 { used as f64 / d.total as f64 } else { 0.0 };
        let pct = (ratio * 100.0).round() as u16;

        // Color by severity
        let color = if pct < 70 { Color::Green } else if pct < 90 { Color::Yellow } else { Color::Red };

        // 1) Title line (name left, usage right), inside its own little block
        let title = format!(
            "{} {}   {} / {}  ({}%)",
            disk_icon(&d.name),
            truncate_middle(&d.name, (slot.width.saturating_sub(6)) as usize / 2),
            human(used),
            human(d.total),
            pct
        );

        // Card frame (thin border per disk)
        let card = Block::default().borders(Borders::ALL).title(title);

        // Render card covering the whole 3-row slot
        f.render_widget(card, *slot);

        // 2) Gauge on the second line inside the card
        // Compute an inner rect (strip card borders), then pick the middle line for the bar
        let inner_card = Rect {
            x: slot.x + 1,
            y: slot.y + 1,
            width: slot.width.saturating_sub(2),
            height: slot.height.saturating_sub(2),
        };
        if inner_card.height == 0 { continue; }

        // Center line for the gauge
        let gauge_rect = Rect {
            x: inner_card.x,
            y: inner_card.y + inner_card.height / 2, // 1 line down inside the card
            width: inner_card.width,
            height: 1,
        };

        let g = Gauge::default()
            .percent(pct)
            .gauge_style(Style::default().fg(color));

        f.render_widget(g, gauge_rect);
    }
}

fn disk_icon(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    if n.contains(":") { "🗄️" }             // network mount
    else if n.contains("nvme") { "⚡" }      // nvme
    else if n.starts_with("sd") { "💽" }     // sata
    else if n.contains("overlay") { "📦" }   // containers/overlayfs
    else { "🖴" }                            // generic drive
}

// Optional helper to keep device names tidy in the title
fn truncate_middle(s: &str, max: usize) -> String {
    if s.len() <= max { return s.to_string(); }
    if max <= 3 { return "...".into(); }
    let keep = max - 3;
    let left = keep / 2;
    let right = keep - left;
    format!("{}...{}", &s[..left], &s[s.len()-right..])
}


fn draw_net_spark(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    title: &str,
    hist: &VecDeque<u64>,
    color: Color,
) {
    let max_points = area.width.saturating_sub(2) as usize;
    let start = hist.len().saturating_sub(max_points);
    let data: Vec<u64> = hist.iter().skip(start).cloned().collect();

    let spark = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title(title.to_string()))
        .data(&data)
        .style(Style::default().fg(color));
    f.render_widget(spark, area);
}

fn draw_top_processes(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    let Some(mm) = m else {
        f.render_widget(Block::default().borders(Borders::ALL).title("Top Processes"), area);
        return;
    };

    let total_mem_bytes = mm.mem_total.max(1); // avoid div-by-zero
    let title = format!("Top Processes ({} total)", mm.process_count);

    // Precompute peak CPU to highlight the hog
    let peak_cpu = mm.top_processes.iter().map(|p| p.cpu_usage).fold(0.0_f32, f32::max);

    // Build rows with per-cell coloring + zebra striping
    let rows: Vec<Row> = mm.top_processes.iter().enumerate().map(|(i, p)| {
        let mem_pct = (p.mem_bytes as f64 / total_mem_bytes as f64) * 100.0;

        // Color helpers
        let cpu_fg = match p.cpu_usage {
            x if x < 25.0 => Color::Green,
            x if x < 60.0 => Color::Yellow,
            _ => Color::Red,
        };
        let mem_fg = match mem_pct {
            x if x < 5.0 => Color::Blue,
            x if x < 20.0 => Color::Magenta,
            _ => Color::Red,
        };

        // Light zebra striping (only foreground shift to avoid loud backgrounds)
        let zebra = if i % 2 == 0 { Style::default().fg(Color::Gray) } else { Style::default() };

        // Emphasize the single top CPU row
        let emphasis = if (p.cpu_usage - peak_cpu).abs() < f32::EPSILON {
            Style::default().add_modifier(Modifier::BOLD)
        } else { Style::default() };

        Row::new(vec![
            Cell::from(p.pid.to_string()).style(Style::default().fg(Color::DarkGray)),
            Cell::from(p.name.clone()),
            Cell::from(format!("{:.1}%", p.cpu_usage)).style(Style::default().fg(cpu_fg)),
            Cell::from(human(p.mem_bytes)),
            Cell::from(format!("{:.2}%", mem_pct)).style(Style::default().fg(mem_fg)),
        ])
        .style(zebra.patch(emphasis))
    }).collect();

    let header = Row::new(vec!["PID", "Name", "CPU %", "Mem", "Mem %"])
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

    let table = Table::new(
            rows,
            vec![
                Constraint::Length(8),         // PID
                Constraint::Percentage(40),    // Name
                Constraint::Length(8),         // CPU %
                Constraint::Length(12),        // Mem
                Constraint::Length(8),         // Mem %
            ],
        )
        .header(header)
        .column_spacing(1)
        .block(Block::default().borders(Borders::ALL).title(title));

    f.render_widget(table, area);
}

