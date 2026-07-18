use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::App;

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

    frame.render_widget(
        Paragraph::new(cpu_summary).block(Block::default().borders(Borders::ALL).title("Overview")),
        body,
    );

    frame.render_widget(
        Paragraph::new("q: quit").style(Style::default().fg(Color::DarkGray)),
        footer,
    );
}
