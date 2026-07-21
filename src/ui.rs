use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::{App, SortKey};
use crate::config::Theme;
use crate::sparkline;

fn load_color(theme: &Theme, pct: f32) -> ratatui::style::Color {
    if pct < 50.0 {
        theme.cpu_low
    } else if pct < 80.0 {
        theme.cpu_mid
    } else {
        theme.cpu_high
    }
}

/// Caps a list of `total` items to fit within `available_rows`, reserving
/// one row for a truncation notice ("+N more") when not everything fits.
fn fit_rows(total: usize, available_rows: usize) -> (usize, bool) {
    if total > available_rows {
        (available_rows.saturating_sub(1), true)
    } else {
        (total.min(available_rows), false)
    }
}

fn format_rate(bytes_per_sec: f64) -> String {
    format!("{}/s", format_bytes_value(bytes_per_sec))
}

fn format_bytes_value(mut value: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    let theme = &app.theme;

    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    frame.render_widget(
        Paragraph::new("tokimono").style(Style::default().fg(theme.accent).bg(theme.background)),
        header,
    );

    // Left column: CPU overview, network, and disk panels, stacked.
    // Right column: the process table, which gets whatever width is left.
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Min(45)]).areas(body);

    // Cap the CPU overview so the other left-column panels always keep some
    // room, even on machines with many cores.
    let core_rows = app.latest.cpu_usage_per_core.len().min(8) as u16;
    let overview_len = core_rows + 2 /* summary rows */ + 2 /* borders */;
    // No border box at all when there are no GPUs and nothing's wrong (the
    // common case — most machines have no NVIDIA hardware) rather than
    // reserving space for an empty panel — draw_gpu's own zero-height guard
    // means a Length(0) area here just isn't rendered. A single status row
    // when NVIDIA is present but something's actually broken (e.g. the
    // kernel module isn't loaded), since that's worth surfacing.
    let gpu_len = if !app.latest.gpus.is_empty() {
        app.latest.gpus.len().min(4) as u16 + 1 /* header row */ + 2 /* borders */
    } else if app.latest.gpu_error.is_some() {
        1 + 2 /* borders */
    } else {
        0
    };
    let network_len = app.latest.networks.len().min(6) as u16 + 1 /* header row */ + 2 /* borders */;
    let [overview_area, gpu_area, network_area, disk_area] = Layout::vertical([
        Constraint::Length(overview_len),
        Constraint::Length(gpu_len),
        Constraint::Length(network_len),
        Constraint::Min(0),
    ])
    .areas(left);

    draw_overview(frame, app, overview_area);
    draw_gpu(frame, app, gpu_area);
    draw_network(frame, app, network_area);
    draw_disk(frame, app, disk_area);
    draw_processes(frame, app, right);

    let footer_text = if let Some(pid) = app.confirm_kill {
        format!("Kill PID {pid}? y = confirm, any other key = cancel")
    } else if let Some(buf) = &app.filter_input {
        format!("Filter: {buf}█  Enter: apply  Esc: cancel")
    } else {
        "q: quit  j/k: select  c/m/p/n: sort  /: filter  x: kill  [ ]: nice  t: tree  h/l: fold"
            .to_string()
    };
    frame.render_widget(
        Paragraph::new(footer_text).style(Style::default().fg(theme.muted).bg(theme.background)),
        footer,
    );
}

fn draw_overview(frame: &mut Frame, app: &App, body: Rect) {
    let theme = &app.theme;
    let cpu_line = if app.latest.cpu_usage_per_core.is_empty() {
        "collecting...".to_string()
    } else {
        let avg = app.latest.cpu_usage_per_core.iter().sum::<f32>()
            / app.latest.cpu_usage_per_core.len() as f32;
        let load = &app.latest.load_avg;
        let temp = match app.latest.cpu_temp {
            Some(t) => format!(" {t:.0}°C"),
            None => String::new(),
        };
        format!(
            "CPU: {avg:.1}%{temp}  LOAD: {:.2} {:.2} {:.2}",
            load.one, load.five, load.fifteen,
        )
    };
    let mem_line = format!(
        "MEM: {:.1}/{:.1} GB  {}",
        app.latest.memory_used as f64 / 1_073_741_824.0,
        app.latest.memory_total as f64 / 1_073_741_824.0,
        if app.latest.swap_total == 0 {
            "SWAP: none".to_string()
        } else {
            format!(
                "SWAP: {:.1}/{:.1} GB",
                app.latest.swap_used as f64 / 1_073_741_824.0,
                app.latest.swap_total as f64 / 1_073_741_824.0,
            )
        }
    );

    let overview_block = Block::default()
        .borders(Borders::ALL)
        .title("Overview")
        .style(Style::default().bg(theme.background));
    let inner = overview_block.inner(body);
    frame.render_widget(overview_block, body);

    let n_cores = app.latest.cpu_usage_per_core.len();
    let max_rows = inner.height.saturating_sub(2) as usize; // rows 0-1 = summary
    let (shown_cores, truncated) = fit_rows(n_cores, max_rows);

    let mut constraints = vec![Constraint::Length(1), Constraint::Length(1)];
    constraints.extend(std::iter::repeat_n(Constraint::Length(1), shown_cores));
    if truncated {
        constraints.push(Constraint::Length(1));
    }
    let rows = Layout::vertical(constraints).split(inner);

    frame.render_widget(
        Paragraph::new(cpu_line).style(Style::default().bg(theme.background)),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(mem_line).style(Style::default().bg(theme.background)),
        rows[1],
    );

    for idx in 0..shown_cores {
        let row = rows[2 + idx];
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
            Paragraph::new(text).style(
                Style::default()
                    .fg(load_color(theme, pct))
                    .bg(theme.background),
            ),
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
            Paragraph::new(text).style(Style::default().fg(theme.muted).bg(theme.background)),
            rows[2 + shown_cores],
        );
    }
}

fn draw_gpu(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .title("GPU")
        .style(Style::default().bg(theme.background));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    if app.latest.gpus.is_empty() {
        if let Some(err) = &app.latest.gpu_error {
            frame.render_widget(
                Paragraph::new(format!("NVIDIA error: {err}"))
                    .style(Style::default().fg(theme.cpu_high).bg(theme.background)),
                inner,
            );
        }
        return;
    }

    let rows_available = inner.height.saturating_sub(1) as usize; // row 0 = header
    let (shown, truncated) = fit_rows(app.latest.gpus.len(), rows_available);

    let constraints = std::iter::repeat_n(Constraint::Length(1), 1 + shown + truncated as usize)
        .collect::<Vec<_>>();
    let rows = Layout::vertical(constraints).split(inner);

    let header = format!(
        "{:<10.10} {:>5} {:>9}/{:<9} {:>5}",
        "NAME", "UTIL", "MEM", "TOTAL", "TEMP"
    );
    frame.render_widget(
        Paragraph::new(header).style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .bg(theme.background),
        ),
        rows[0],
    );

    for (row_idx, gpu) in app.latest.gpus.iter().take(shown).enumerate() {
        let temp = match gpu.temp {
            Some(t) => format!("{t:.0}°C"),
            None => "-".to_string(),
        };
        let text = format!(
            "{:<10.10} {:>4.0}% {:>9}/{:<9} {:>5}",
            gpu.name,
            gpu.utilization_pct,
            format_bytes_value(gpu.memory_used as f64),
            format_bytes_value(gpu.memory_total as f64),
            temp,
        );
        frame.render_widget(
            Paragraph::new(text).style(Style::default().bg(theme.background)),
            rows[1 + row_idx],
        );
    }

    if truncated {
        let text = format!("… +{} more GPUs", app.latest.gpus.len() - shown);
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(theme.muted).bg(theme.background)),
            rows[1 + shown],
        );
    }
}

fn draw_network(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Network")
        .style(Style::default().bg(theme.background));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    let rows_available = inner.height.saturating_sub(1) as usize; // row 0 = header
    let (shown, truncated) = fit_rows(app.latest.networks.len(), rows_available);

    let constraints = std::iter::repeat_n(Constraint::Length(1), 1 + shown + truncated as usize)
        .collect::<Vec<_>>();
    let rows = Layout::vertical(constraints).split(inner);

    let header = format!("{:<10.10} {:>10} {:>10}", "IFACE", "DOWN", "UP");
    frame.render_widget(
        Paragraph::new(header).style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .bg(theme.background),
        ),
        rows[0],
    );

    for (row_idx, net) in app.latest.networks.iter().take(shown).enumerate() {
        let text = format!(
            "{:<10.10} {:>10} {:>10}",
            net.name,
            format_rate(net.rx_rate),
            format_rate(net.tx_rate),
        );
        frame.render_widget(
            Paragraph::new(text).style(Style::default().bg(theme.background)),
            rows[1 + row_idx],
        );
    }

    if truncated {
        let text = format!("… +{} more interfaces", app.latest.networks.len() - shown);
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(theme.muted).bg(theme.background)),
            rows[1 + shown],
        );
    }
}

fn draw_disk(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Disks")
        .style(Style::default().bg(theme.background));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    let rows_available = inner.height.saturating_sub(1) as usize; // row 0 = header
    let (shown, truncated) = fit_rows(app.latest.disks.len(), rows_available);

    let constraints = std::iter::repeat_n(Constraint::Length(1), 1 + shown + truncated as usize)
        .collect::<Vec<_>>();
    let rows = Layout::vertical(constraints).split(inner);

    let header = format!(
        "{:<10.10} {:>5} {:>9} {:>9}",
        "MOUNT", "USED", "READ", "WRITE"
    );
    frame.render_widget(
        Paragraph::new(header).style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .bg(theme.background),
        ),
        rows[0],
    );

    for (row_idx, disk) in app.latest.disks.iter().take(shown).enumerate() {
        let used_pct = if disk.total_space == 0 {
            0.0
        } else {
            let used = disk.total_space.saturating_sub(disk.available_space);
            used as f64 / disk.total_space as f64 * 100.0
        };
        let text = format!(
            "{:<10.10} {:>4.0}% {:>9} {:>9}",
            disk.mount_point,
            used_pct,
            format_rate(disk.read_rate),
            format_rate(disk.write_rate),
        );
        frame.render_widget(
            Paragraph::new(text).style(Style::default().bg(theme.background)),
            rows[1 + row_idx],
        );
    }

    if truncated {
        let text = format!("… +{} more disks", app.latest.disks.len() - shown);
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(theme.muted).bg(theme.background)),
            rows[1 + shown],
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
    let theme = &app.theme;
    let visible = app.visible_processes();
    let show_tree = app.tree_view && app.filter.is_empty();
    let dir = if app.sort_desc { "↓" } else { "↑" };
    let mut title = if app.filter.is_empty() {
        format!(
            "Processes ({} {dir}, {} total)",
            sort_label(app.sort_key),
            visible.len()
        )
    } else {
        format!(
            "Processes ({} {dir}, {}/{} matching \"{}\")",
            sort_label(app.sort_key),
            visible.len(),
            app.latest.processes.len(),
            app.filter,
        )
    };
    if show_tree {
        title.push_str(" · tree");
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Style::default().bg(theme.background));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    let rows_available = inner.height.saturating_sub(1) as usize; // row 0 = column header
    let selected_idx = app
        .selected_pid
        .and_then(|pid| visible.iter().position(|r| r.process.pid == pid))
        .unwrap_or(0);
    let start = if selected_idx >= rows_available {
        selected_idx + 1 - rows_available
    } else {
        0
    };
    let end = (start + rows_available).min(visible.len());

    let constraints =
        std::iter::repeat_n(Constraint::Length(1), 1 + (end - start)).collect::<Vec<_>>();
    let rows = Layout::vertical(constraints).split(inner);

    let header = format!(
        "{:>7} {:<24.24} {:>7} {:>10} {:>5}",
        "PID", "NAME", "CPU%", "MEM", "NICE"
    );
    frame.render_widget(
        Paragraph::new(header).style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .bg(theme.background),
        ),
        rows[0],
    );

    for (row_idx, row) in visible[start..end].iter().enumerate() {
        let process = row.process;
        let is_selected = Some(process.pid) == app.selected_pid;
        let nice = process
            .nice
            .map(|n| n.to_string())
            .unwrap_or_else(|| "-".to_string());
        let name = if show_tree {
            let indent = "  ".repeat(row.depth);
            let marker = if row.has_children {
                if app.is_collapsed(process.pid) {
                    "▸ "
                } else {
                    "▾ "
                }
            } else {
                "  "
            };
            format!("{indent}{marker}{}", process.name)
        } else {
            process.name.clone()
        };
        let text = format!(
            "{:>7} {:<24.24} {:>6.1}% {:>9.1}M {:>5}",
            process.pid,
            name,
            process.cpu_usage,
            process.memory as f64 / 1_048_576.0,
            nice,
        );
        let style = if is_selected {
            Style::default()
                .bg(theme.selection_bg)
                .fg(theme.selection_fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(theme.background)
        };
        frame.render_widget(Paragraph::new(text).style(style), rows[1 + row_idx]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_kilobyte_has_no_decimal() {
        assert_eq!(format_rate(0.0), "0 B/s");
        assert_eq!(format_rate(512.0), "512 B/s");
    }

    #[test]
    fn kilobyte_boundary_rounds_up_a_unit() {
        assert_eq!(format_rate(1024.0), "1.0 KB/s");
    }

    #[test]
    fn fractional_kilobytes_keep_one_decimal() {
        assert_eq!(format_rate(1536.0), "1.5 KB/s");
    }

    #[test]
    fn megabyte_and_gigabyte_units() {
        assert_eq!(format_rate(1_048_576.0), "1.0 MB/s");
        assert_eq!(format_rate(1_073_741_824.0), "1.0 GB/s");
    }

    #[test]
    fn caps_at_terabytes_instead_of_growing_forever() {
        let huge = 1024f64.powi(5);
        assert_eq!(format_rate(huge), "1024.0 TB/s");
    }

    #[test]
    fn fit_rows_reserves_a_truncation_row_when_over_capacity() {
        assert_eq!(fit_rows(10, 4), (3, true));
    }

    #[test]
    fn fit_rows_uses_full_list_when_it_fits() {
        assert_eq!(fit_rows(3, 4), (3, false));
        assert_eq!(fit_rows(4, 4), (4, false));
    }
}
