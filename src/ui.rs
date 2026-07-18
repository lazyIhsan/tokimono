use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::{App, SortKey};
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

    // Cap the CPU overview so the process table always keeps some room,
    // even on machines with many cores.
    let core_rows = app.latest.cpu_usage_per_core.len().min(8) as u16;
    let overview_len = core_rows + 1 /* summary row */ + 2 /* borders */;
    let [overview_area, process_area] =
        Layout::vertical([Constraint::Length(overview_len), Constraint::Min(0)]).areas(body);

    draw_overview(frame, app, overview_area);
    draw_processes(frame, app, process_area);

    let footer_text = if let Some(pid) = app.confirm_kill {
        format!("Kill PID {pid}? y = confirm, any other key = cancel")
    } else {
        "q: quit  j/k: select  c/m/p/n: sort  x: kill".to_string()
    };
    frame.render_widget(
        Paragraph::new(footer_text).style(Style::default().fg(Color::DarkGray)),
        footer,
    );
}

fn draw_overview(frame: &mut Frame, app: &App, body: Rect) {
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
}

fn sort_label(key: SortKey) -> &'static str {
    match key {
        SortKey::Cpu => "CPU%",
        SortKey::Memory => "MEM",
        SortKey::Pid => "PID",
        SortKey::Name => "NAME",
    }
}

fn draw_processes(frame: &mut Frame, app: &App, area: Rect) {
    let dir = if app.sort_desc { "↓" } else { "↑" };
    let title = format!(
        "Processes ({} {dir}, {} total)",
        sort_label(app.sort_key),
        app.latest.processes.len()
    );
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    let rows_available = inner.height.saturating_sub(1) as usize; // row 0 = column header
    let selected_idx = app
        .selected_pid
        .and_then(|pid| app.latest.processes.iter().position(|p| p.pid == pid))
        .unwrap_or(0);
    let start = if selected_idx >= rows_available {
        selected_idx + 1 - rows_available
    } else {
        0
    };
    let end = (start + rows_available).min(app.latest.processes.len());

    let constraints =
        std::iter::repeat_n(Constraint::Length(1), 1 + (end - start)).collect::<Vec<_>>();
    let rows = Layout::vertical(constraints).split(inner);

    let header = format!("{:>7} {:<24.24} {:>7} {:>10}", "PID", "NAME", "CPU%", "MEM");
    frame.render_widget(
        Paragraph::new(header).style(Style::default().add_modifier(Modifier::BOLD)),
        rows[0],
    );

    for (row_idx, process) in app.latest.processes[start..end].iter().enumerate() {
        let is_selected = Some(process.pid) == app.selected_pid;
        let text = format!(
            "{:>7} {:<24.24} {:>6.1}% {:>9.1}M",
            process.pid,
            process.name,
            process.cpu_usage,
            process.memory as f64 / 1_048_576.0,
        );
        let style = if is_selected {
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        frame.render_widget(Paragraph::new(text).style(style), rows[1 + row_idx]);
    }
}
