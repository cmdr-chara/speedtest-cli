use crate::i18n::ui;
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

    let lower_height = if app.is_complete() { 9 } else { 4 };
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
        render_completion_panel(frame, app, vertical[3]);
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
            Line::from(ui(" SPEEDTEST ")).style(
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
        ui(label),
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
        metric_line("DOWNLOAD", format_speed(app.download_mbps), DOWNLOAD_ACCENT),
        metric_line("PING", format_ms(app.ping_ms), Color::White),
        metric_line("LOADED ↓", format_ms(app.download_loaded_ms), Color::White),
    ]);
    let right = Paragraph::new(vec![
        metric_line("UPLOAD", format_speed(app.upload_mbps), UPLOAD_ACCENT),
        metric_line("JITTER", format_ms(app.jitter_ms), Color::White),
        metric_line("LOADED ↑", format_ms(app.upload_loaded_ms), Color::White),
    ]);
    frame.render_widget(left, metrics[0]);
    frame.render_widget(right, metrics[1]);
}

fn render_sparkline(frame: &mut Frame, app: &App, area: Rect, accent: Color) {
    let width = area.width.saturating_sub(2) as usize;
    let data = resample_sparkline(&app.samples, width);
    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .title(Line::from(Span::styled(
                    ui(" THROUGHPUT TRACE "),
                    Style::default().fg(Color::DarkGray),
                )))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .data(&data)
        .style(Style::default().fg(accent));
    frame.render_widget(sparkline, area);
}

fn resample_sparkline(samples: &std::collections::VecDeque<f64>, width: usize) -> Vec<u64> {
    if samples.is_empty() || width == 0 {
        return Vec::new();
    }
    let source = samples.iter().copied().collect::<Vec<_>>();
    if source.len() == 1 {
        return vec![source[0].max(0.0) as u64; width];
    }

    (0..width)
        .map(|column| {
            let position = if width <= 1 {
                0.0
            } else {
                column as f64 * (source.len() - 1) as f64 / (width - 1) as f64
            };
            let left = position.floor() as usize;
            let right = position.ceil() as usize;
            let fraction = position - left as f64;
            let value = source[left] * (1.0 - fraction) + source[right] * fraction;
            value.max(0.0) as u64
        })
        .collect()
}

fn render_completion_panel(frame: &mut Frame, app: &App, area: Rect) {
    let Some(result) = app.result.as_ref() else {
        frame.render_widget(legacy_completion(app), area);
        return;
    };
    let Some(analysis) = result.analysis.as_ref() else {
        frame.render_widget(legacy_completion(app), area);
        return;
    };

    let quality = &analysis.quality;
    let quality_color = if quality.is_s_tier() {
        S_TIER_ACCENT
    } else {
        grade_color(quality.grade)
    };
    let buffer_grade = quality.bufferbloat.grade.map_or("—", QualityGrade::label);
    let jitter_p95 = analysis
        .latency
        .jitter
        .as_ref()
        .map_or(0.0, |jitter| jitter.p95_ms);

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(3)])
        .split(area);
    let cards = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(vertical[0]);

    let tier = quality
        .tier_label()
        .map_or(String::new(), |tier| format!("  ◆ {tier}"));
    let quality_card = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                ui(format!("{}/100 {}", quality.score, quality.grade.label())),
                Style::default()
                    .fg(quality_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(tier, Style::default().fg(S_TIER_ACCENT)),
        ]),
        Line::from(Span::styled(
            ui(format!("{} confidence", quality.confidence.label())),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(ui(format!(
            "Bufferbloat {buffer_grade}   ↓ {}   ↑ {}",
            format_delta(quality.bufferbloat.download_increase_ms),
            format_delta(quality.bufferbloat.upload_increase_ms)
        ))),
    ])
    .block(card_block(" QUALITY ", quality_color));

    let workloads = Paragraph::new(vec![
        workload_line("Gaming", quality.workloads.gaming),
        workload_line("Calls", quality.workloads.video_calls),
        workload_line("Streaming", quality.workloads.streaming),
    ])
    .block(card_block(" WORKLOADS ", Color::Gray));

    let tails = Paragraph::new(vec![
        Line::from(ui(format!(
            "Idle p95      {:>6.1} ms",
            analysis.latency.idle.p95_ms
        ))),
        Line::from(ui(format!(
            "Idle p99      {:>6.1} ms",
            analysis.latency.idle.p99_ms
        ))),
        Line::from(ui(format!("Jitter p95    {:>6.1} ms", jitter_p95))),
    ])
    .block(card_block(" TAIL LATENCY ", Color::Gray));

    frame.render_widget(quality_card, cards[0]);
    frame.render_widget(workloads, cards[1]);
    frame.render_widget(tails, cards[2]);

    let diagnosis = if let Some(finding) = quality.findings.first() {
        let recommendation = finding
            .recommendation
            .as_deref()
            .unwrap_or(finding.evidence.as_str());
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    ui(format!(" {} ", finding.severity.label())),
                    Style::default()
                        .fg(severity_color(finding.severity))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    finding.title.clone(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(Span::styled(
                ui(format!("  {recommendation}")),
                Style::default().fg(Color::Gray),
            )),
        ])
    } else {
        Paragraph::new(vec![
            Line::from(Span::styled(
                ui(" HEALTHY "),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                ui("  No material network-quality finding was detected in this run."),
                Style::default().fg(Color::Gray),
            )),
        ])
    };
    frame.render_widget(
        diagnosis
            .block(card_block(" DIAGNOSIS ", Color::Gray))
            .wrap(Wrap { trim: true }),
        vertical[1],
    );
}

fn card_block(title: &'static str, color: Color) -> Block<'static> {
    Block::default()
        .title(Line::from(Span::styled(
            ui(title),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
}

fn workload_line(label: &'static str, grade: QualityGrade) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            ui(format!("{label:<13}")),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            grade.label(),
            Style::default()
                .fg(grade_color(grade))
                .add_modifier(Modifier::BOLD),
        ),
    ])
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
    let tier = quality
        .tier_label()
        .map_or(String::new(), |tier| format!(" ◆ {tier}"));

    Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                ui(format!(
                    "QUALITY {}/100 {}{tier}",
                    quality.score,
                    quality.grade.label()
                )),
                Style::default()
                    .fg(quality_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(ui("  •  ")),
            Span::raw(ui(format!(
                "game {}  calls {}  stream {}",
                quality.workloads.gaming.label(),
                quality.workloads.video_calls.label(),
                quality.workloads.streaming.label()
            ))),
        ]),
        Line::from(ui(format!(
            "buffer {}  ↓ {}  ↑ {}",
            quality.bufferbloat.grade.map_or("—", QualityGrade::label),
            format_delta(quality.bufferbloat.download_increase_ms),
            format_delta(quality.bufferbloat.upload_increase_ms)
        ))),
    ])
    .alignment(Alignment::Center)
}

fn legacy_completion(app: &App) -> Paragraph<'static> {
    let download = app.download_mbps.unwrap_or_default();
    let upload = app.upload_mbps.unwrap_or_default();
    Paragraph::new(Line::from(Span::styled(
        ui(format!("result  ↓ {download:.1} Mbps   ↑ {upload:.1} Mbps")),
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
        Span::styled(ui("Cloudflare Edge"), Style::default().fg(Color::Gray)),
        Span::raw(ui("  •  ")),
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

fn metric_line(label: &'static str, value: String, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            ui(format!(" {label:<10}")),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            value,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
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
