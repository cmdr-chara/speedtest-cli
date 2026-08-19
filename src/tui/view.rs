use ratatui::{
    prelude::{
        Alignment, Color, Constraint, Direction, Frame, Layout, Line, Modifier, Rect, Span, Style,
    },
    widgets::{Block, Borders, Paragraph, Sparkline, Wrap},
};

use crate::model::{FindingSeverity, QualityGrade, TestPhase};

use super::{app::App, speedometer};

const DOWNLOAD_ACCENT: Color = Color::Cyan;
const UPLOAD_ACCENT: Color = Color::Magenta;
const COMPLETE_ACCENT: Color = Color::Green;
const S_TIER_ACCENT: Color = Color::LightCyan;
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

    let lower_height = if app.is_complete() { 6 } else { 3 };
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(10),
            Constraint::Length(4),
            Constraint::Length(lower_height),
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
        frame.render_widget(completion_panel(app), vertical[3]);
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

    if app.is_complete() {
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(7),
                Constraint::Length(4),
                Constraint::Length(2),
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
        frame.render_widget(compact_quality(app), vertical[3]);
        frame.render_widget(footer(app), vertical[4]);
    } else {
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
        TestPhase::Complete => ("NETWORK ANALYSIS COMPLETE", COMPLETE_ACCENT),
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

fn completion_panel(app: &App) -> Paragraph<'static> {
    let Some(result) = app.result.as_ref() else {
        return legacy_completion(app);
    };
    let Some(analysis) = result.analysis.as_ref() else {
        return legacy_completion(app);
    };

    let quality = &analysis.quality;
    let buffer_grade = quality.bufferbloat.grade.map_or("—", QualityGrade::label);
    let jitter_p95 = analysis
        .latency
        .jitter
        .as_ref()
        .map_or(0.0, |jitter| jitter.p95_ms);
    let finding = quality.findings.first();
    let quality_color = if quality.is_s_tier() {
        S_TIER_ACCENT
    } else {
        grade_color(quality.grade)
    };

    let mut quality_line = vec![
        Span::styled(
            format!(" QUALITY {}/100 {} ", quality.score, quality.grade.label()),
            Style::default()
                .fg(quality_color)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if quality.is_s_tier() {
        quality_line.push(Span::styled(
            " ◆ S-TIER ",
            Style::default()
                .fg(S_TIER_ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
    }
    quality_line.push(Span::styled(
        format!("{} confidence", quality.confidence.label()),
        Style::default().fg(Color::DarkGray),
    ));

    let mut lines = vec![
        Line::from(quality_line),
        Line::from(vec![
            Span::styled(" Gaming ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                quality.workloads.gaming.label(),
                Style::default().fg(grade_color(quality.workloads.gaming)),
            ),
            Span::styled("   Calls ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                quality.workloads.video_calls.label(),
                Style::default().fg(grade_color(quality.workloads.video_calls)),
            ),
            Span::styled("   Streaming ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                quality.workloads.streaming.label(),
                Style::default().fg(grade_color(quality.workloads.streaming)),
            ),
            Span::styled("   Cloud gaming ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                quality.workloads.cloud_gaming.label(),
                Style::default().fg(grade_color(quality.workloads.cloud_gaming)),
            ),
        ]),
        Line::from(format!(
            " tails  idle p95 {:.1} ms  p99 {:.1} ms  •  jitter p95 {:.1} ms",
            analysis.latency.idle.p95_ms, analysis.latency.idle.p99_ms, jitter_p95
        )),
        Line::from(format!(
            " bufferbloat {buffer_grade}  ↓ {}  ↑ {}",
            format_delta(quality.bufferbloat.download_increase_ms),
            format_delta(quality.bufferbloat.upload_increase_ms)
        )),
    ];

    if let Some(finding) = finding {
        let recommendation = finding
            .recommendation
            .as_deref()
            .unwrap_or(finding.evidence.as_str());
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} ", finding.severity.label()),
                Style::default()
                    .fg(severity_color(finding.severity))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} — {recommendation}", finding.title),
                Style::default().fg(Color::Gray),
            ),
        ]));
    }

    Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
}

fn compact_quality(app: &App) -> Paragraph<'static> {
    let Some(quality) = app
        .result
        .as_ref()
        .and_then(|result| result.analysis.as_ref())
        .map(|analysis| &analysis.quality)
    else {
        return legacy_completion(app);
    };

    let quality_color = if quality.is_s_tier() {
        S_TIER_ACCENT
    } else {
        grade_color(quality.grade)
    };
    let tier = quality.tier_label().map_or(String::new(), |tier| format!(" ◆ {tier}"));

    Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                format!(
                    "QUALITY {}/100 {}{tier}",
                    quality.score,
                    quality.grade.label()
                ),
                Style::default()
                    .fg(quality_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  •  "),
            Span::raw(format!(
                "game {}  calls {}  stream {}",
                quality.workloads.gaming.label(),
                quality.workloads.video_calls.label(),
                quality.workloads.streaming.label()
            )),
        ]),
        Line::from(format!(
            "buffer {}  ↓ {}  ↑ {}",
            quality.bufferbloat.grade.map_or("—", QualityGrade::label),
            format_delta(quality.bufferbloat.download_increase_ms),
            format_delta(quality.bufferbloat.upload_increase_ms)
        )),
    ])
    .alignment(Alignment::Center)
}

fn legacy_completion(app: &App) -> Paragraph<'static> {
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

fn grade_color(grade: QualityGrade) -> Color {
    match grade {
        QualityGrade::APlus | QualityGrade::A => Color::Green,
        QualityGrade::B => Color::Cyan,
        QualityGrade::C | QualityGrade::D => Color::Yellow,
        QualityGrade::F => Color::Red,
    }
}

fn severity_color(severity: FindingSeverity) -> Color {
    match severity {
        FindingSeverity::Info => Color::Cyan,
        FindingSeverity::Warning => Color::Yellow,
        FindingSeverity::Critical => Color::Red,
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

fn format_delta(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_string(), |value| format!("+{value:.1} ms"))
}
