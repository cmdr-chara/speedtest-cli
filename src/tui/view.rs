use ratatui::{
    prelude::{
        Alignment, Color, Constraint, Direction, Frame, Layout, Line, Modifier, Rect, Span, Style,
    },
    widgets::{Block, Borders, Paragraph, Sparkline},
};

use crate::model::TestPhase;

use super::{app::App, speedometer};

const DOWNLOAD_ACCENT: Color = Color::Cyan;
const UPLOAD_ACCENT: Color = Color::Magenta;
const COMPLETE_ACCENT: Color = Color::Green;
const COMPACT_WIDTH: u16 = 72;
const COMPACT_HEIGHT: u16 = 23;

pub(super) fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < COMPACT_WIDTH || area.height < COMPACT_HEIGHT {
        draw_compact(frame, app);
    } else {
        draw_full(frame, app);
    }
}

fn draw_full(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let outer = shell();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(11),
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(phase_header(app), vertical[0]);

    let accent = gauge_accent(app.phase);
    speedometer::render(
        frame,
        vertical[1],
        &app.speedometer,
        accent,
        shows_speed(app.phase),
    );

    render_metrics(frame, app, vertical[2]);
    if app.is_complete() {
        frame.render_widget(completion_strip(app), vertical[3]);
    } else {
        render_sparkline(frame, app, vertical[3], accent);
    }
    frame.render_widget(footer(app), vertical[4]);
}

fn draw_compact(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let outer = shell();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(7),
            Constraint::Length(4),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(phase_header(app), vertical[0]);
    speedometer::render(
        frame,
        vertical[1],
        &app.speedometer,
        gauge_accent(app.phase),
        shows_speed(app.phase),
    );
    render_metrics(frame, app, vertical[2]);
    frame.render_widget(footer(app), vertical[3]);
}

fn shell() -> Block<'static> {
    Block::default()
        .title(
            Line::from(" SPEEDTEST ").style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
}

fn phase_header(app: &App) -> Paragraph<'static> {
    let (label, color) = match app.phase {
        TestPhase::Complete => ("TEST COMPLETE", COMPLETE_ACCENT),
        phase => (phase.label(), gauge_accent(phase)),
    };

    Paragraph::new(Line::from(Span::styled(
        label,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )))
    .alignment(Alignment::Center)
}

fn render_metrics(frame: &mut Frame, app: &App, area: Rect) {
    let metrics = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let left = Paragraph::new(vec![
        metric_line("DOWNLOAD", format_speed(app.download_mbps)),
        metric_line("PING", format_ms(app.ping_ms)),
        metric_line("LOADED ↓", format_ms(app.download_loaded_ms)),
    ]);
    let right = Paragraph::new(vec![
        metric_line("UPLOAD", format_speed(app.upload_mbps)),
        metric_line("JITTER", format_ms(app.jitter_ms)),
        metric_line("LOADED ↑", format_ms(app.upload_loaded_ms)),
    ]);
    frame.render_widget(left, metrics[0]);
    frame.render_widget(right, metrics[1]);
}

fn render_sparkline(frame: &mut Frame, app: &App, area: Rect, accent: Color) {
    let data: Vec<u64> = app
        .samples
        .iter()
        .map(|value| value.max(0.0) as u64)
        .collect();
    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .data(&data)
        .style(Style::default().fg(accent));
    frame.render_widget(sparkline, area);
}

fn completion_strip(app: &App) -> Paragraph<'static> {
    let download = app.download_mbps.unwrap_or_default();
    let upload = app.upload_mbps.unwrap_or_default();
    Paragraph::new(Line::from(Span::styled(
        format!("result  ↓ {download:.1} Mbps   ↑ {upload:.1} Mbps"),
        Style::default().fg(Color::Gray),
    )))
    .block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray)),
    )
    .alignment(Alignment::Center)
}

fn footer(app: &App) -> Paragraph<'static> {
    let instruction = if app.is_complete() {
        "enter / q / esc to close"
    } else {
        "q / esc / ctrl-c to cancel"
    };

    Paragraph::new(Line::from(vec![
        Span::styled("Cloudflare Edge", Style::default().fg(Color::Gray)),
        Span::raw("  •  "),
        Span::styled(instruction, Style::default().fg(Color::DarkGray)),
    ]))
    .alignment(Alignment::Center)
}

fn shows_speed(phase: TestPhase) -> bool {
    matches!(
        phase,
        TestPhase::Download | TestPhase::Upload | TestPhase::Complete
    )
}

fn gauge_accent(phase: TestPhase) -> Color {
    match phase {
        TestPhase::Upload => UPLOAD_ACCENT,
        _ => DOWNLOAD_ACCENT,
    }
}

fn metric_line(label: &'static str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {label:<10}"),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(value, Style::default().fg(Color::White)),
    ])
}

fn format_speed(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_string(), |value| format!("{value:.1} Mbps"))
}

fn format_ms(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_string(), |value| format!("{value:.1} ms"))
}
