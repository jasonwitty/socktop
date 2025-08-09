//! Network sparklines (download/upload).

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Sparkline},
};
use std::collections::VecDeque;

pub fn draw_net_spark(
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
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title.to_string()),
        )
        .data(&data)
        .style(Style::default().fg(color));
    f.render_widget(spark, area);
}
