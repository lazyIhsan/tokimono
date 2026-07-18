use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::App;
use crate::sparkline;

fn load_color(pct: f32) -> Color {
    if pct < 50.0 {
        Color::Green
    } else if pct < 80.0 {
        Color::Yellow
    } else {
        Color::Red
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    frame.render_widget(
        Paragraph::new("tokimono").style(Style::default().fg(Color::Cyan)),
        header,
    );

    let cpu_summary = if app.latest.cpu_usage_per_core.is_empty() {
        "collecting...".to_string()
    } else {
        let avg = app.latest.cpu_usage_per_core.iter().sum::<f32>()
            / app.latest.cpu_usage_per_core.len() as f32;
        format!(
            "CPU: {avg:.1}%  MEM: {:.1} / {:.1} GB",
            app.latest.memory_used as f64 / 1_073_741_824.0,
            app.latest.memory_total as f64 / 1_073_741_824.0,
        )
    };

    let overview_block = Block::default().borders(Borders::ALL).title("Overview");
    let inner = overview_block.inner(body);
    frame.render_widget(overview_block, body);

    let n_cores = app.latest.cpu_usage_per_core.len();
    let max_rows = inner.height.saturating_sub(1) as usize; // row 0 = summary
    let truncated = n_cores > max_rows;
    let shown_cores = if truncated {
        max_rows.saturating_sub(1)
    } else {
        n_cores.min(max_rows)
    };

    let mut constraints = vec![Constraint::Length(1)];
    constraints.extend(std::iter::repeat_n(Constraint::Length(1), shown_cores));
    if truncated {
        constraints.push(Constraint::Length(1));
    }
    let rows = Layout::vertical(constraints).split(inner);

    frame.render_widget(Paragraph::new(cpu_summary), rows[0]);

    for idx in 0..shown_cores {
        let row = rows[1 + idx];
        let pct = app
            .latest
            .cpu_usage_per_core
            .get(idx)
            .copied()
            .unwrap_or(0.0);
        let spark_w = row.width.saturating_sub(10) as usize;
        let samples_needed = spark_w * 2;
        let start = app.cpu_history.len().saturating_sub(samples_needed);
        let samples: Vec<f32> = app
            .cpu_history
            .iter()
            .skip(start)
            .map(|snap| snap.get(idx).copied().unwrap_or(0.0))
            .collect();
        let spark = sparkline::render(&samples, 100.0);
        let text = format!("{idx:>2} {spark} {pct:>5.1}%");
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(load_color(pct))),
            row,
        );
    }

    if truncated {
        let remaining = &app.latest.cpu_usage_per_core[shown_cores..];
        let remaining_avg = remaining.iter().sum::<f32>() / remaining.len() as f32;
        let text = format!(
            "… +{} more cores (avg {:.1}%)",
            n_cores - shown_cores,
            remaining_avg
        );
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(Color::DarkGray)),
            rows[1 + shown_cores],
        );
    }

    frame.render_widget(
        Paragraph::new("q: quit").style(Style::default().fg(Color::DarkGray)),
        footer,
    );
}
