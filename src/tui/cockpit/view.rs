use std::time::Duration;

use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::Modifier,
    symbols::Marker,
    text::{Line, Span, Text},
    widgets::{
        Axis, Block, BorderType, Borders, Cell, Chart, Clear, Dataset, GraphType, Padding,
        Paragraph, Row, Table, Tabs, Wrap,
    },
    Frame,
};

use super::{
    services::{Tool, HISTORY_DAYS},
    state::{Activity, Cockpit, Load, Modal, Screen, SECTIONS},
    theme::Theme,
};
use crate::{
    model::{FindingSeverity, QualityGrade, TestPhase, TestResult},
    output,
    tui::{numerals, speedometer},
};

const BRAND: [&str; 3] = [
    "█▀▀ █▀█ █▀▀ █▀▀ █▀▄ ▀█▀ █▀▀ █▀▀ ▀█▀",
    "▀▀█ █▀▀ █▀  █▀  █ █  █  █▀  ▀▀█  █ ",
    "▄▄█ █   █▄▄ █▄▄ █▄▀  █  █▄▄ ▄▄█  █ ",
];

pub(super) fn draw(frame: &mut Frame, app: &mut Cockpit, theme: Theme, elapsed: Duration) {
    let area = frame.area();
    frame.render_widget(Block::default().style(theme.base()), area);
    if area.width < 80 || area.height < 24 {
        small(frame, app, theme, area);
    } else {
        let inner = workspace(area).inner(Margin::new(2, 1));
        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(inner);
        chrome(frame, app, theme, rows[0], rows[1]);
        match app.screen() {
            Screen::Home => home(frame, app, theme, rows[2]),
            Screen::Configure | Screen::Settings => configure(frame, app, theme, rows[2]),
            Screen::Live => live(frame, app, theme, rows[2], elapsed),
            Screen::Results => results(frame, app, theme, rows[2]),
            Screen::History => history(frame, app, theme, rows[2]),
            Screen::Statistics => statistics(frame, app, theme, rows[2]),
            Screen::Compare => compare(frame, app, theme, rows[2]),
            Screen::Dns | Screen::Diagnostics => tools(frame, app, theme, rows[2]),
            Screen::Tool => report(frame, app, theme, rows[2], elapsed),
            Screen::Failure => failure(frame, app, theme, rows[2]),
        }
        footer(frame, app, theme, rows[3]);
    }
    if let Some(modal) = app.modal {
        overlay(frame, modal, theme, area);
    }
}

/// Keep related information together on ultrawide/maximized terminals. A TUI
/// cannot resize the emulator font; responsive composition and multi-cell values
/// improve hierarchy without changing the user's profile or window size.
pub(super) fn workspace(area: Rect) -> Rect {
    let width = area.width.min(120);
    let height = area.height.min(38);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

fn metric_height(app: &Cockpit, area: Rect) -> u16 {
    if !app.compact && area.width >= 90 && area.height >= 19 {
        6
    } else {
        3
    }
}

/// Highlight the label, never its description or trailing whitespace.
fn choice(label: &str, selected: bool, t: Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(if selected { "› " } else { "  " }, t.focus()),
        Span::styled(
            format!(" {label} "),
            if selected { t.selected() } else { t.strong() },
        ),
    ])
}

fn single(value: impl AsRef<str>) -> String {
    output::safe_text(value.as_ref()).replace(['\n', '\t'], " ")
}

fn chrome(frame: &mut Frame, app: &Cockpit, t: Theme, head: Rect, tabs: Rect) {
    let columns =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(head);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("SPEEDTEST", t.focus()),
                Span::styled(" / NETWORK COCKPIT", t.muted()),
            ]),
            Line::styled(
                app.pages
                    .iter()
                    .map(|page| page.screen.title())
                    .collect::<Vec<_>>()
                    .join(" / "),
                t.muted(),
            ),
        ]),
        columns[0],
    );
    let (status, color) = match app.activity {
        Some(Activity::Test) => ("● MEASURING", t.focus),
        Some(Activity::Saving) => ("● SAVING", t.warning),
        Some(Activity::Tool) => ("● DIAGNOSTIC RUNNING", t.focus),
        None if app.latest().is_some() => ("○ LAST RESULT AVAILABLE", t.success),
        None => ("○ NETWORK NOT PROBED", t.muted),
    };
    frame.render_widget(
        Paragraph::new(status)
            .style(t.base().fg(color))
            .alignment(Alignment::Right),
        columns[1],
    );
    let index = app
        .pages
        .iter()
        .rev()
        .find_map(|p| SECTIONS.iter().position(|s| *s == p.screen))
        .unwrap_or(0);
    let labels = [
        "Home",
        "Test",
        "History",
        "Stats",
        "DNS",
        "Diagnostics",
        "Settings",
    ];
    let titles: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            Line::from(if i == index {
                format!("> {label}")
            } else {
                label.to_string()
            })
        })
        .collect();
    frame.render_widget(
        Tabs::new(titles)
            .select(index)
            .style(t.muted())
            .highlight_style(t.selected())
            .divider(" ")
            .padding("", " ")
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(t.base().fg(t.line)),
            ),
        tabs,
    );
}

fn heading(frame: &mut Frame, title: &str, subtitle: &str, t: Theme, area: Rect) -> Rect {
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(title, t.focus()),
            Line::styled(single(subtitle), t.muted()),
        ])
        .wrap(Wrap { trim: true }),
        rows[0],
    );
    rows[1]
}

fn home(frame: &mut Frame, app: &Cockpit, t: Theme, area: Rect) {
    let rows = Layout::vertical([Constraint::Length(5), Constraint::Min(1)]).split(area);
    let hero =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(rows[0]);
    let mut logo: Vec<_> = BRAND.iter().map(|s| Line::styled(*s, t.focus())).collect();
    logo.push(Line::styled("Your network, in focus.", t.strong()));
    frame.render_widget(Paragraph::new(logo), hero[0]);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled("MEASUREMENT PROFILE", t.strong()),
            Line::styled(app.options.backend_label(), t.focus()),
            Line::styled(
                format!(
                    "{} s / phase  ·  {} streams",
                    app.options.duration, app.options.streams
                ),
                t.muted(),
            ),
            Line::styled("No background network probes", t.muted()),
        ]),
        hero[1],
    );
    let spacious = !app.compact && rows[1].height >= 18;
    let columns = Layout::horizontal([
        Constraint::Length(if area.width >= 96 { 34 } else { 29 }),
        Constraint::Min(1),
    ])
    .split(rows[1]);
    let labels = [
        ("Run Speed Test", "Configure, then start"),
        ("History", "Browse saved measurements"),
        ("Statistics", "Trends and comparisons"),
        ("DNS Tools", "Inspect and compare resolvers"),
        ("Diagnostics", "Find connection problems"),
        ("Settings", "Appearance and test defaults"),
    ];
    let mut y = columns[0].y;
    for (index, (label, description)) in labels.iter().enumerate() {
        let selected = app.page().selected == index;
        frame.render_widget(
            Paragraph::new(choice(label, selected, t)),
            Rect::new(columns[0].x, y, columns[0].width.saturating_sub(2), 1),
        );
        if spacious || index == 0 {
            frame.render_widget(
                Paragraph::new(*description).style(t.muted()),
                Rect::new(
                    columns[0].x + 3,
                    y + 1,
                    columns[0].width.saturating_sub(5),
                    1,
                ),
            );
            y += 3;
        } else {
            y += 1;
        }
    }
    let divider = Block::default()
        .borders(Borders::LEFT)
        .border_style(t.base().fg(t.line))
        .padding(Padding::new(3, 0, 0, 0));
    let detail = divider.inner(columns[1]);
    frame.render_widget(divider, columns[1]);
    if let Some(result) = app.latest() {
        let metric_rows = if spacious { 6 } else { 3 };
        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(metric_rows),
            Constraint::Min(1),
        ])
        .split(detail);
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled("LATEST RESULT", t.strong()),
                Line::styled(
                    format!(
                        "{} UTC · {}",
                        result.timestamp.format("%d %b %H:%M"),
                        single(&result.backend)
                    ),
                    t.muted(),
                ),
            ]),
            rows[0],
        );
        let metrics = Layout::horizontal([Constraint::Percentage(50); 2]).split(rows[1]);
        metric(
            frame,
            "DOWNLOAD",
            &format!("{:.1}", result.download.mbps),
            "Mbps",
            t,
            metrics[0],
        );
        metric(
            frame,
            "UPLOAD",
            &format!("{:.1}", result.upload.mbps),
            "Mbps",
            t,
            metrics[1],
        );
        let mut lines = vec![Line::from(format!(
            "Idle {:.1} ms · jitter {:.1} ms",
            result.latency.idle_ms, result.latency.jitter_ms
        ))];
        if let Some(analysis) = &result.analysis {
            lines.push(Line::styled(
                format!(
                    "Quality {}/100 · {} · {} confidence",
                    analysis.quality.score,
                    analysis.quality.grade.label(),
                    analysis.quality.confidence.label()
                ),
                t.strong().fg(grade_color(analysis.quality.grade, t)),
            ));
            if spacious {
                lines.push(Line::default());
                if let Some(finding) = analysis.quality.findings.first() {
                    lines.push(Line::styled(
                        single(&finding.title),
                        t.strong().fg(severity_color(finding.severity, t)),
                    ));
                    lines.push(Line::from(single(&finding.evidence)));
                } else {
                    lines.push(Line::from(
                        "No findings in this result. This is not a continuous connection monitor.",
                    ));
                }
                lines.push(Line::default());
            }
        }
        lines.push(Line::styled("v  Open result", t.focus()));
        frame.render_widget(
            Paragraph::new(lines)
                .style(t.base())
                .wrap(Wrap { trim: true }),
            rows[2],
        );
    } else {
        let (title, text, color) = match &app.history {
            Load::Loading => ("READING LOCAL HISTORY", "Loading saved results. No network activity.", t.text),
            Load::Failed(_) => ("HISTORY UNAVAILABLE", "Local history could not be read. Open History for details, or press r to retry. You can still run a test.", t.warning),
            Load::Ready(_) => ("No tests yet", "Start your first test to see throughput, latency and connection quality here.\n\nYour connection has not been probed.", t.text),
        };
        let mut lines = vec![
            Line::styled("RECENT ACTIVITY", t.strong()),
            Line::default(),
            Line::styled(title, t.strong().fg(color)),
            Line::default(),
        ];
        lines.extend(text.lines().map(Line::from));
        frame.render_widget(
            Paragraph::new(lines)
                .style(t.base())
                .wrap(Wrap { trim: true }),
            detail,
        );
    }
}

fn metric(frame: &mut Frame, label: &str, value: &str, unit: &str, t: Theme, area: Rect) {
    frame.render_widget(
        Paragraph::new(label).style(t.strong()),
        Rect::new(area.x, area.y, area.width, 1),
    );
    if area.height >= 5
        && numerals::draw(
            frame,
            Rect::new(area.x, area.y + 1, area.width.saturating_sub(1), 3),
            value,
            t.strong(),
            Alignment::Left,
        )
    {
        frame.render_widget(
            Paragraph::new(unit).style(t.muted()),
            Rect::new(area.x, area.y + 4, area.width, 1),
        );
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(value.to_owned(), t.strong()),
                Span::styled(format!(" {unit}"), t.muted()),
            ])),
            Rect::new(
                area.x,
                area.y + 1,
                area.width,
                area.height.saturating_sub(1),
            ),
        );
    }
}

fn configure(frame: &mut Frame, app: &mut Cockpit, t: Theme, area: Rect) {
    let config = app.screen() == Screen::Configure;
    let area = heading(
        frame,
        if config {
            "READY WHEN YOU ARE"
        } else {
            "MAKE IT YOURS"
        },
        "Session settings only · Enter or +/- edits a value · Tab changes section",
        t,
        area,
    );
    let columns =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(area);
    let mut fields: Vec<(&str, String)> = Vec::new();
    if config {
        fields.push(("Start test", "Enter to begin".into()));
    }
    fields.extend([
        ("Backend", app.options.backend_label().into()),
        (
            "Duration / phase",
            format!("{} seconds", app.options.duration),
        ),
        ("Concurrent streams", app.options.streams.to_string()),
        ("Render cap", format!("{} FPS", app.options.fps)),
        (
            "Save to history",
            if app.options.no_save { "OFF" } else { "ON" }.into(),
        ),
        (
            "Overall deadline",
            format!("{} seconds", app.options.timeout),
        ),
        (
            "Reduced motion",
            if app.reduced_motion { "ON" } else { "OFF" }.into(),
        ),
        ("Balanced preset", "8s / 2 / 60 FPS".into()),
        ("Color palette", app.palette.label().into()),
        (
            "Layout",
            if app.compact {
                "Compact"
            } else {
                "Comfortable"
            }
            .into(),
        ),
    ]);
    let spacing = u16::from(!app.compact && columns[0].height >= (fields.len() * 2) as u16);
    let rows: Vec<_> = fields
        .iter()
        .enumerate()
        .map(|(index, (label, value))| {
            let selected = index == app.page().selected;
            Row::new(vec![
                Cell::from(choice(label, selected, t)),
                Cell::from(value.clone()).style(if selected { t.focus() } else { t.strong() }),
            ])
            .bottom_margin(spacing)
            .style(t.base())
        })
        .collect();
    frame.render_widget(
        Table::new(
            rows,
            [Constraint::Percentage(55), Constraint::Percentage(45)],
        )
        .column_spacing(1),
        columns[0],
    );
    let frame_block = Block::default()
        .borders(Borders::LEFT)
        .border_style(t.base().fg(t.line))
        .padding(Padding::new(2, 0, 0, 0));
    let info = frame_block.inner(columns[1]);
    frame.render_widget(frame_block, columns[1]);
    let selected = app.page().selected;
    let index = selected.checked_sub(usize::from(config));
    let (title, description) = match index {
        None => ("BEFORE YOU START", "This test saturates your connection and can use substantial data.\n\nIdle latency → download → upload → quality analysis."),
        Some(0) => ("MEASUREMENT BACKEND", "Choose Cloudflare or LibreSpeed. Different server paths can produce different results. No connection is made until you start."),
        Some(1) => ("DURATION PER PHASE", "Seconds for each download and upload phase, from 3 to 30. Idle latency and preparation take extra time. Longer tests transfer more data."),
        Some(2) => ("CONCURRENT STREAMS", "Use 1 to 16 concurrent transfers. More streams can help saturate a fast link but also increase server and local load."),
        Some(3) => ("RENDER CAP", "Maximum frames per second, not the measurement rate. A 60 FPS cap is usually enough; lowering it reduces terminal work."),
        Some(4) => ("LOCAL HISTORY", "Save completed tests for History and Statistics. Turning this off keeps new results in this session; explicit --output exports still run."),
        Some(5) => ("OVERALL DEADLINE", "Maximum total seconds for a started test or diagnostic. Time spent browsing this menu does not count."),
        Some(6) => ("REDUCED MOTION", "Use direct values and static activity indicators instead of an animated needle. Measurements are unchanged."),
        Some(7) => ("BALANCED PRESET", "Restore 8-second phases, 2 streams, 60 FPS and a 120-second deadline. Backend, history, appearance and export settings are preserved."),
        Some(8) => ("TERMINAL COLORS", "Terminal (adaptive) uses your terminal's foreground, background and ANSI palette on Linux, macOS and Windows.\n\nGraphite and Light are optional fixed palettes. Monochrome uses only your default text colors. Nothing modifies the terminal profile."),
        _ => ("READABILITY", "Comfortable uses large metric digits and extra row spacing where space allows. Compact uses ordinary text.\n\nFor larger body text, use your terminal's Zoom In or profile font-size setting. The app reflows after resizing and never changes your terminal font."),
    };
    let mut lines = vec![Line::styled(title, t.focus()), Line::default()];
    lines.extend(description.lines().map(Line::from));
    lines.extend([
        Line::default(),
        Line::styled("Enter / + / -  Change selected value", t.strong()),
        Line::default(),
    ]);
    if app.options.output.is_some() {
        lines.push(Line::styled(
            "Explicit export enabled",
            t.base().fg(t.warning),
        ));
        lines.push(Line::from(
            "--output is written after each test, even with history off. The same path is reused.",
        ));
    } else {
        lines.push(Line::styled(
            if app.options.no_save {
                "History OFF · results stay in this session"
            } else {
                "History ON · completed tests are saved locally"
            },
            t.muted(),
        ));
    }
    if app.options.librespeed_server.is_some() {
        lines.push(Line::from(
            "Custom LibreSpeed endpoint configured; used when LibreSpeed is selected.",
        ));
    }
    scroll(frame, app, t, info, lines);
}

fn live(frame: &mut Frame, app: &Cockpit, t: Theme, area: Rect, elapsed: Duration) {
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(8),
        Constraint::Length(3),
    ])
    .split(area);
    let phase = app.live.phase;
    let mut rail = Vec::new();
    for (index, step) in [
        TestPhase::Preparing,
        TestPhase::Latency,
        TestPhase::Download,
        TestPhase::Upload,
    ]
    .iter()
    .enumerate()
    {
        let active = *step == phase;
        rail.push(Span::styled(
            format!(
                "{} {}{}  ",
                index + 1,
                if active { "› " } else { "" },
                step.label()
            ),
            if active { t.focus() } else { t.muted() },
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(rail)), rows[0]);
    let columns = Layout::horizontal([
        Constraint::Percentage(64),
        Constraint::Length(3),
        Constraint::Min(1),
    ])
    .split(rows[1]);
    let gauge = Rect::new(
        columns[0].x + columns[0].width.saturating_sub(70) / 2,
        columns[0].y,
        columns[0].width.min(70),
        columns[0].height.min(22),
    );
    speedometer::render_themed(
        frame,
        gauge,
        &app.live.speedometer,
        matches!(
            phase,
            TestPhase::Download | TestPhase::Upload | TestPhase::Complete
        ),
        speedometer::GaugePalette {
            background: t.background,
            accent: t.focus,
            text: t.text,
            secondary: t.muted,
            track: t.line,
        },
        !app.compact,
    );
    let side = columns[2];
    let spacious = !app.compact && side.height >= 21;
    let mut lines = vec![
        Line::styled(
            if app.activity == Some(Activity::Saving) {
                "FINISHING"
            } else {
                phase.label()
            },
            t.focus(),
        ),
        Line::styled(
            format!("Elapsed {}s · latest samples", elapsed.as_secs()),
            t.muted(),
        ),
        Line::default(),
    ];
    let details = if spacious {
        frame.render_widget(
            Paragraph::new(lines).style(t.base()),
            Rect::new(side.x, side.y, side.width, 3),
        );
        metric(
            frame,
            "DOWNLOAD",
            &app.live
                .download_mbps
                .map_or("—".into(), |v| format!("{v:.1}")),
            "Mbps",
            t,
            Rect::new(side.x, side.y + 3, side.width, 6),
        );
        metric(
            frame,
            "UPLOAD",
            &app.live
                .upload_mbps
                .map_or("—".into(), |v| format!("{v:.1}")),
            "Mbps",
            t,
            Rect::new(side.x, side.y + 9, side.width, 6),
        );
        lines = Vec::new();
        Rect::new(side.x, side.y + 15, side.width, side.height - 15)
    } else {
        lines.extend([
            Line::from(format!("Down   {}", speed(app.live.download_mbps))),
            Line::from(format!("Up     {}", speed(app.live.upload_mbps))),
            Line::default(),
        ]);
        side
    };
    lines.extend([
        Line::from(format!("Idle   {}", ms(app.live.ping_ms))),
        Line::from(format!("Jitter {}", ms(app.live.jitter_ms))),
        Line::from(format!("Load ↓ {}", ms(app.live.download_loaded_ms))),
        Line::from(format!("Load ↑ {}", ms(app.live.upload_loaded_ms))),
    ]);
    frame.render_widget(Paragraph::new(lines).style(t.base()), details);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                format!(
                    "{} · {} streams · {}s per transfer phase",
                    app.options.backend_label(),
                    app.options.streams,
                    app.options.duration
                ),
                t.muted(),
            ),
            Line::styled(
                "Gauge is smoothed; samples are provisional. Final results use completed measurements.",
                t.muted(),
            ),
        ]),
        rows[2],
    );
}

fn results(frame: &mut Frame, app: &mut Cockpit, t: Theme, area: Rect) {
    let Some(result) = app.result.as_ref() else {
        return;
    };
    let subtitle = format!(
        "{} UTC · {} · {}",
        result.timestamp.format("%d %b %Y %H:%M:%S"),
        single(&result.backend),
        single(&result.server.host)
    );
    let area = heading(frame, "MEASUREMENT COMPLETE", &subtitle, t, area);
    let height = metric_height(app, area);
    let rows = Layout::vertical([Constraint::Length(height), Constraint::Min(1)]).split(area);
    result_metrics(frame, result, t, rows[0]);
    let roomy = height == 6 && area.width >= 100 && rows[1].height >= 12;
    let mut summary = vec![
        Line::styled(
            if roomy && app.save_notice.starts_with("SAVE FAILED") {
                "SAVE FAILED · See details →".into()
            } else {
                single(&app.save_notice)
            },
            t.strong()
                .fg(if app.save_notice.starts_with("SAVE FAILED") {
                    t.error
                } else {
                    t.text
                }),
        ),
        Line::default(),
    ];
    if roomy {
        summary.extend([
            Line::styled("LOADED LATENCY", t.strong()),
            Line::from(format!(
                "Download  {}",
                ms(result.latency.download_loaded_ms)
            )),
            Line::from(format!("Upload    {}", ms(result.latency.upload_loaded_ms))),
            Line::default(),
        ]);
    } else {
        summary.push(Line::from(format!(
            "Loaded latency   Download {}   /   Upload {}",
            ms(result.latency.download_loaded_ms),
            ms(result.latency.upload_loaded_ms)
        )));
    }
    let mut findings = Vec::new();
    if roomy && app.save_notice.starts_with("SAVE FAILED") {
        findings.extend([
            Line::styled("SAVE FAILED", t.strong().fg(t.error)),
            Line::from(single(&app.save_notice)),
            Line::default(),
        ]);
    }
    if let Some(analysis) = &result.analysis {
        let q = &analysis.quality;
        summary.push(Line::styled(
            format!(
                "QUALITY  {}/100  {}{}",
                q.score,
                q.grade.label(),
                if q.is_s_tier() { " / S-TIER" } else { "" }
            ),
            t.strong().fg(grade_color(q.grade, t)),
        ));
        summary.push(Line::from(format!(
            "{} confidence · bufferbloat {}",
            q.confidence.label(),
            q.bufferbloat.grade.map_or("n/a", QualityGrade::label)
        )));
        summary.push(Line::from(format!(
            "Idle p95 {:.1} / p99 {:.1} ms",
            analysis.latency.idle.p95_ms, analysis.latency.idle.p99_ms
        )));
        if roomy {
            summary.extend([
                Line::default(),
                Line::styled("APPLICATION READINESS", t.strong()),
                Line::from(format!("Gaming        {}", q.workloads.gaming.label())),
                Line::from(format!("Video calls   {}", q.workloads.video_calls.label())),
                Line::from(format!("Streaming     {}", q.workloads.streaming.label())),
                Line::from(format!(
                    "Cloud gaming  {}",
                    q.workloads.cloud_gaming.label()
                )),
            ]);
        } else {
            summary.push(Line::from(format!(
                "Gaming {}   Calls {}   Streaming {}   Cloud gaming {}",
                q.workloads.gaming.label(),
                q.workloads.video_calls.label(),
                q.workloads.streaming.label(),
                q.workloads.cloud_gaming.label()
            )));
        }
        if q.findings.is_empty() {
            findings.extend([
                Line::styled("NO FINDINGS", t.strong().fg(t.success)),
                Line::from("No issues were flagged by the local analysis of this result."),
            ]);
        }
        for finding in &q.findings {
            findings.push(Line::styled(
                format!("{} · {}", finding.severity.label(), single(&finding.title)),
                t.strong().fg(severity_color(finding.severity, t)),
            ));
            findings.push(Line::from(single(&finding.evidence)));
            if let Some(recommendation) = &finding.recommendation {
                findings.push(Line::default());
                findings.push(Line::from(single(recommendation)));
            }
            findings.push(Line::default());
        }
    } else {
        findings.push(Line::from(
            "Quality analysis unavailable for this saved result.",
        ));
    }
    findings.push(Line::styled(
        "Mbps = decimal megabits/s. HTTP latency is not ICMP; scores are local heuristics.",
        t.muted(),
    ));
    if roomy {
        let columns = Layout::horizontal([
            Constraint::Length(34),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(rows[1]);
        frame.render_widget(
            Paragraph::new(summary)
                .style(t.base())
                .wrap(Wrap { trim: true }),
            columns[0],
        );
        frame.render_widget(
            Block::default()
                .borders(Borders::LEFT)
                .border_style(t.base().fg(t.line)),
            columns[1],
        );
        scroll(frame, app, t, columns[2], findings);
    } else {
        summary.push(Line::default());
        summary.extend(findings);
        scroll(frame, app, t, rows[1], summary);
    }
}

fn result_metrics(frame: &mut Frame, result: &TestResult, t: Theme, area: Rect) {
    let columns = Layout::horizontal([Constraint::Percentage(25); 4]).split(area);
    for (rect, label, value, unit) in [
        (columns[0], "DOWNLOAD", result.download.mbps, "Mbps"),
        (columns[1], "UPLOAD", result.upload.mbps, "Mbps"),
        (columns[2], "IDLE LATENCY", result.latency.idle_ms, "ms"),
        (columns[3], "JITTER", result.latency.jitter_ms, "ms"),
    ] {
        metric(frame, label, &format!("{value:.1}"), unit, t, rect);
    }
}

fn archive_state(frame: &mut Frame, app: &mut Cockpit, t: Theme, area: Rect) -> bool {
    let state = match &app.history {
        Load::Loading => Some(("READING LOCAL HISTORY", "Loading local results. No network activity.".to_owned())),
        Load::Failed(error) => Some(("HISTORY UNAVAILABLE", format!("{}\n\nPress r to retry. No history was changed. You can still run a new test.", single(error)))),
        Load::Ready(archive) if archive.results.is_empty() => Some(("No tests yet", format!("No saved results in the last {HISTORY_DAYS} days.\n\nOpen Test with Tab to create your first baseline. History must be enabled to appear here."))),
        _ => None,
    };
    if let Some((title, message)) = state {
        let mut lines = vec![Line::styled(title, t.focus()), Line::default()];
        lines.extend(message.lines().map(|line| Line::from(line.to_owned())));
        scroll(frame, app, t, area, lines);
        true
    } else {
        false
    }
}

fn history(frame: &mut Frame, app: &mut Cockpit, t: Theme, area: Rect) {
    let area = heading(
        frame,
        "YOUR NETWORK, OVER TIME",
        "Last 30 days · newest first · Enter opens a result · c compares latest two",
        t,
        area,
    );
    if archive_state(frame, app, t, area) {
        return;
    }
    let Load::Ready(archive) = &app.history else {
        return;
    };
    let preview = !app.compact && area.height >= 19;
    let layout = Layout::vertical([
        Constraint::Min(5),
        Constraint::Length(if preview { 9 } else { 0 }),
    ])
    .split(area);
    let rows: Vec<_> = archive
        .results
        .iter()
        .rev()
        .map(|r| {
            let quality = r.analysis.as_ref().map_or("n/a".into(), |a| {
                format!("{} {}", a.quality.score, a.quality.grade.label())
            });
            Row::new(vec![
                Cell::from(r.timestamp.format("%m-%d %H:%M UTC").to_string()),
                Cell::from(single(&r.backend)),
                right(format!("{:.1}", r.download.mbps)),
                right(format!("{:.1}", r.upload.mbps)),
                right(format!("{:.1}", r.latency.idle_ms)),
                right(quality),
            ])
        })
        .collect();
    app.table.select(Some(app.page().selected));
    frame.render_stateful_widget(
        Table::new(
            rows,
            [
                Constraint::Length(if area.width >= 100 { 19 } else { 15 }),
                Constraint::Length(if area.width >= 100 { 13 } else { 11 }),
                Constraint::Fill(1),
                Constraint::Fill(1),
                Constraint::Fill(1),
                Constraint::Length(9),
            ],
        )
        .header(
            Row::new(vec![
                Cell::from("DATE / UTC"),
                Cell::from("BACKEND"),
                right("DOWN Mbps".into()),
                right("UP Mbps".into()),
                right("IDLE ms".into()),
                right("QUALITY".into()),
            ])
            .style(t.strong())
            .bottom_margin(1),
        )
        .row_highlight_style(t.selected())
        .highlight_symbol("› ")
        .column_spacing(1)
        .style(t.base()),
        layout[0],
        &mut app.table,
    );
    if preview {
        if let Some(selected) = archive.results.iter().rev().nth(app.page().selected) {
            let block = Block::default()
                .borders(Borders::TOP)
                .border_style(t.base().fg(t.line))
                .title(" SELECTED RESULT ")
                .title_style(t.strong());
            let detail = block.inner(layout[1]);
            frame.render_widget(block, layout[1]);
            let parts =
                Layout::vertical([Constraint::Length(2), Constraint::Length(6)]).split(detail);
            frame.render_widget(
                Paragraph::new(format!(
                    "{} UTC · {} · Enter opens details",
                    selected.timestamp.format("%d %b %Y %H:%M"),
                    single(&selected.backend)
                ))
                .style(t.muted()),
                parts[0],
            );
            result_metrics(frame, selected, t, parts[1]);
        }
    }
}

fn right(value: String) -> Cell<'static> {
    Cell::from(Line::from(value).alignment(Alignment::Right))
}

fn statistics(frame: &mut Frame, app: &mut Cockpit, t: Theme, area: Rect) {
    let area = heading(
        frame,
        "THE BIGGER PICTURE",
        "30-day local history · Enter compares the latest two saved runs",
        t,
        area,
    );
    if archive_state(frame, app, t, area) {
        return;
    }
    let Load::Ready(archive) = &app.history else {
        return;
    };
    let Some(summary) = &archive.summary else {
        return;
    };
    let rows = Layout::vertical([
        Constraint::Length(metric_height(app, area)),
        Constraint::Min(1),
    ])
    .split(area);
    let columns = Layout::horizontal([
        Constraint::Percentage(34),
        Constraint::Percentage(33),
        Constraint::Percentage(33),
    ])
    .split(rows[0]);
    metric(
        frame,
        "MEDIAN DOWNLOAD",
        &format!("{:.1}", summary.median_download_mbps),
        "Mbps",
        t,
        columns[0],
    );
    metric(
        frame,
        "MEDIAN UPLOAD",
        &format!("{:.1}", summary.median_upload_mbps),
        "Mbps",
        t,
        columns[1],
    );
    metric(
        frame,
        "MEDIAN LATENCY",
        &format!("{:.1}", summary.median_ping_ms),
        "ms",
        t,
        columns[2],
    );
    let plot = !app.compact && rows[1].height >= 14;
    let parts = Layout::vertical([
        Constraint::Length(if plot { 8 } else { 0 }),
        Constraint::Min(1),
    ])
    .split(rows[1]);
    if plot {
        let data: Vec<_> = archive
            .results
            .iter()
            .rev()
            .take(60)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .enumerate()
            .map(|(index, result)| (index as f64, result.download.mbps))
            .collect();
        if data.len() >= 2 {
            let maximum = data
                .iter()
                .fold(1.0f64, |maximum, (_, value)| maximum.max(*value));
            frame.render_widget(
                Chart::new(vec![Dataset::default()
                    .data(&data)
                    .marker(Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(t.base().fg(t.focus))])
                .style(t.base())
                .block(
                    Block::default()
                        .title("Download · saved samples, not continuous monitoring")
                        .title_style(t.strong()),
                )
                .x_axis(
                    Axis::default()
                        .bounds([0.0, (data.len() - 1) as f64])
                        .labels(vec![Line::from("oldest"), Line::from("latest")])
                        .style(t.base().fg(t.line)),
                )
                .y_axis(
                    Axis::default()
                        .bounds([0.0, maximum])
                        .labels(vec![
                            Line::from("0"),
                            Line::from(format!("{maximum:.1} Mbps")),
                        ])
                        .style(t.base().fg(t.line)),
                ),
                parts[0],
            );
        } else {
            frame.render_widget(
                Paragraph::new(
                    "ONE SAVED SAMPLE\n\nSave another test to see a download history chart.",
                )
                .style(t.base()),
                parts[0],
            );
        }
    }
    let mut lines = vec![
        Line::styled(
            format!(
                "{} RUNS  ·  TREND: {}",
                summary.runs,
                summary.trend.label().to_uppercase()
            ),
            t.focus(),
        ),
        Line::styled(
            if plot {
                String::new()
            } else {
                summary.download_sparkline.clone()
            },
            t.focus(),
        ),
        Line::from(format!(
            "Best down {:.1} Mbps  /  best up {:.1} Mbps  /  p95 idle {:.1} ms",
            summary.best_download_mbps, summary.best_upload_mbps, summary.p95_ping_ms
        )),
        Line::from(format!(
            "Median quality {}  /  S-tier runs {}",
            summary
                .median_quality_score
                .map_or("n/a".into(), |n| format!("{n:.0}/100")),
            summary.s_tier_runs
        )),
        Line::default(),
    ];
    if summary.anomalies.is_empty() {
        lines.push(Line::styled(
            "No anomaly flags in this sample. This is not a guarantee of stability.",
            t.muted(),
        ));
    }
    for anomaly in &summary.anomalies {
        lines.push(Line::styled(
            format!(
                "{} · {}",
                anomaly.severity.label(),
                single(&anomaly.message)
            ),
            t.base().fg(t.warning),
        ));
    }
    scroll(frame, app, t, parts[1], lines);
}

fn compare(frame: &mut Frame, app: &mut Cockpit, t: Theme, area: Rect) {
    let area = heading(
        frame,
        "BEFORE / AFTER",
        "Latest two saved runs · last 30 days · no new measurement",
        t,
        area,
    );
    if archive_state(frame, app, t, area) {
        return;
    }
    let Load::Ready(archive) = &app.history else {
        return;
    };
    let Some(c) = &archive.comparison else {
        frame.render_widget(Paragraph::new("Two saved tests are needed for a comparison.\n\nRun another test with history enabled, then press r to reload.").style(t.base()).wrap(Wrap { trim: true }), area);
        return;
    };
    let mut lines = vec![
        Line::styled(
            format!(
                "BEFORE {}  →  AFTER {} UTC",
                c.before_timestamp.format("%d %b %H:%M"),
                c.after_timestamp.format("%d %b %H:%M")
            ),
            t.muted(),
        ),
        Line::default(),
        Line::styled(
            "METRIC             BEFORE        AFTER       CHANGE",
            t.muted(),
        ),
    ];
    for (label, delta, unit) in [
        ("Download", &c.download_mbps, "Mbps"),
        ("Upload", &c.upload_mbps, "Mbps"),
        ("Idle latency", &c.ping_ms, "ms"),
        ("Jitter", &c.jitter_ms, "ms"),
    ] {
        let direction = if delta.absolute_change.abs() < f64::EPSILON {
            "same"
        } else if delta.improved {
            "better"
        } else {
            "worse"
        };
        lines.push(Line::styled(
            format!(
                "{label:<16} {:>9.1}  {:>11.1}  {:+9.1} {unit} ({direction})",
                delta.before, delta.after, delta.absolute_change
            ),
            t.base().fg(if direction == "same" {
                t.text
            } else if delta.improved {
                t.success
            } else {
                t.warning
            }),
        ));
    }
    lines.extend([
        Line::default(),
        Line::styled(single(&c.verdict), t.focus()),
        Line::default(),
        Line::styled(
            "Paths, backends and test conditions can differ; compare like-for-like runs.",
            t.muted(),
        ),
    ]);
    scroll(frame, app, t, area, lines);
}

fn tools(frame: &mut Frame, app: &mut Cockpit, t: Theme, area: Rect) {
    let dns = app.screen() == Screen::Dns;
    let area = heading(
        frame,
        if dns {
            "DNS WORKBENCH"
        } else {
            "DIAGNOSTIC WORKBENCH"
        },
        "Read-only tools · select a tool, then explicitly start it · no automatic probes",
        t,
        area,
    );
    let columns =
        Layout::horizontal([Constraint::Percentage(47), Constraint::Percentage(53)]).split(area);
    let tools = if dns { Tool::DNS } else { Tool::DIAGNOSTICS };
    let selected = tools[app.page().selected];
    let lines: Vec<_> = tools
        .iter()
        .enumerate()
        .flat_map(|(i, tool)| {
            [
                choice(tool.title(), i == app.page().selected, t),
                Line::default(),
            ]
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), columns[0]);
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(t.base().fg(t.line))
        .padding(Padding::new(2, 0, 0, 0));
    let detail = block.inner(columns[1]);
    frame.render_widget(block, columns[1]);
    let mut lines = vec![
        Line::styled(
            if selected.network() {
                "NETWORK CHECK · MANUAL START"
            } else {
                "LOCAL INSPECTION"
            },
            t.focus(),
        ),
        Line::default(),
        Line::from(selected.description()),
        Line::default(),
        Line::styled("Enter  Open tool", t.focus()),
        Line::default(),
    ];
    if dns {
        lines.push(Line::styled("DNS changes stay in the CLI: set, optimize, reset and rollback retain their confirmation/rollback flow.", t.muted()));
    } else {
        lines.push(Line::styled(
            "LAN server/client and advanced command options remain available from the CLI.",
            t.muted(),
        ));
    }
    scroll(frame, app, t, detail, lines);
}

fn report(frame: &mut Frame, app: &mut Cockpit, t: Theme, area: Rect, elapsed: Duration) {
    let Some(tool) = app.tool else {
        return;
    };
    let area = heading(
        frame,
        tool.title(),
        if tool.network() {
            "Network activity only after Start · q / Esc asks before cancellation"
        } else {
            "Local, read-only inspection · existing CLI implementation"
        },
        t,
        area,
    );
    let lines = match &app.report {
        None => vec![
            Line::styled("READY TO START", t.focus()), Line::default(), Line::from(tool.description()), Line::default(),
            choice("Enter  Start diagnostic", true, t), Line::default(),
            Line::styled(format!("Overall deadline: {} seconds. Settings apply to this session.", app.options.timeout), t.muted()),
        ],
        Some(Load::Loading) => {
            let pulse = if app.reduced_motion { "●" } else { ["◐", "◓", "◑", "◒"][(elapsed.as_millis() / 150 % 4) as usize] };
            vec![Line::styled(format!("{pulse} RUNNING  ·  {}s elapsed", elapsed.as_secs()), t.focus()), Line::default(),
                Line::from(tool.description()), Line::default(), Line::styled("The interface stays responsive. Esc cancels with confirmation.", t.muted())]
        }
        Some(Load::Failed(error)) => vec![Line::styled("DIAGNOSTIC FAILED", t.base().fg(t.error).add_modifier(Modifier::BOLD)),
            Line::default(), Line::from(single(error)), Line::default(),
            Line::from("Check connectivity, permissions or availability of native tools. No configuration was changed."),
            Line::default(), Line::styled("Enter / r  Retry    Esc  Back", t.focus())],
        Some(Load::Ready(text)) => {
            let mut lines = vec![Line::styled("COMPLETED · Enter / r runs again", t.base().fg(t.success)), Line::default()];
            lines.extend(output::safe_text(text).lines().map(|line| Line::from(line.to_owned())));
            lines
        }
    };
    scroll(frame, app, t, area, lines);
}

fn failure(frame: &mut Frame, app: &mut Cockpit, t: Theme, area: Rect) {
    let area = heading(
        frame,
        "TEST COULD NOT COMPLETE",
        "No incomplete measurement was saved",
        t,
        area,
    );
    let lines = vec![Line::styled("ERROR", t.base().fg(t.error).add_modifier(Modifier::BOLD)), Line::default(),
        Line::from(single(&app.failure)), Line::default(),
        Line::from("Check connectivity, return to Test to change backend, or increase the deadline in Settings."),
        Line::default(), Line::styled("Enter / r  Retry    Esc  Configuration    Tab  Another section", t.focus())];
    scroll(frame, app, t, area, lines);
}

fn scroll(frame: &mut Frame, app: &mut Cockpit, t: Theme, area: Rect, lines: Vec<Line<'static>>) {
    let paragraph = Paragraph::new(Text::from(lines))
        .style(t.base())
        .wrap(Wrap { trim: false });
    // Exact wrapped heights are essential for navigation after terminal/font zoom.
    let count = paragraph.line_count(area.width);
    let overflow = count > usize::from(area.height) && area.height > 1;
    let visible = area.height.saturating_sub(u16::from(overflow));
    let max = count
        .saturating_sub(usize::from(visible))
        .min(usize::from(u16::MAX)) as u16;
    app.page_mut().scroll = app.page().scroll.min(max);
    frame.render_widget(
        paragraph.scroll((app.page().scroll, 0)),
        Rect::new(area.x, area.y, area.width, visible),
    );
    if overflow {
        frame.render_widget(
            Paragraph::new(format!(
                "↑↓ {}–{} / {} · PgUp/PgDn",
                usize::from(app.page().scroll) + 1,
                (usize::from(app.page().scroll) + usize::from(visible)).min(count),
                count
            ))
            .style(t.muted()),
            Rect::new(area.x, area.y + visible, area.width, 1),
        );
    }
}

fn footer(frame: &mut Frame, app: &Cockpit, t: Theme, area: Rect) {
    let context = if app.activity == Some(Activity::Saving) {
        "Finishing completed result. Quit requests wait for the save.".into()
    } else if app.activity.is_some() {
        "Esc cancel task  ·  q cancel and quit  ·  Ctrl+C stop immediately".into()
    } else if !app.notice.is_empty() {
        single(&app.notice)
    } else {
        match app.screen() {
            Screen::Home => {
                "Enter configure / open  ·  v latest result  ·  r reload local history".into()
            }
            Screen::Configure => {
                "Enter start / edit  ·  +/- change value  ·  settings are session-only".into()
            }
            Screen::Settings => {
                "Enter edit  ·  +/- change value  ·  settings are session-only".into()
            }
            Screen::Live => {
                "Esc cancel test  ·  q cancel and quit  ·  Ctrl+C stop immediately".into()
            }
            Screen::Results => "Enter another test  ·  j/k or PgUp/PgDn scroll details".into(),
            Screen::History => "Enter result  ·  c compare latest two  ·  r reload".into(),
            Screen::Statistics | Screen::Compare => {
                "Enter compare (from Stats)  ·  j/k scroll  ·  r reload".into()
            }
            Screen::Dns | Screen::Diagnostics => {
                "Enter open tool  ·  no network activity on this screen".into()
            }
            Screen::Tool | Screen::Failure => {
                "Enter / r start or retry  ·  j/k or PgUp/PgDn scroll".into()
            }
        }
    };
    let bindings = if app.activity.is_some() {
        Line::from(vec![
            Span::styled("?", t.focus()),
            Span::styled(" help  ·  ", t.muted()),
            Span::styled("q", t.focus()),
            Span::styled(" quit  ·  ", t.muted()),
            Span::styled("Navigation resumes when the task finishes.", t.muted()),
        ])
    } else {
        Line::from(vec![
            Span::styled("j/k", t.focus()),
            Span::styled(" move  ·  ", t.muted()),
            Span::styled("Tab / ←→", t.focus()),
            Span::styled(" sections  ·  ", t.muted()),
            Span::styled("Esc", t.focus()),
            Span::styled(" back  ·  ", t.muted()),
            Span::styled("?", t.focus()),
            Span::styled(" help  ·  ", t.muted()),
            Span::styled("q", t.focus()),
            Span::styled(" quit", t.muted()),
        ])
    };
    frame.render_widget(
        Paragraph::new(vec![Line::styled(context, t.muted()), bindings]),
        area,
    );
}

fn small(frame: &mut Frame, app: &Cockpit, t: Theme, area: Rect) {
    let text = vec![
        Line::styled("SPEEDTEST / NETWORK COCKPIT", t.focus()),
        Line::default(),
        Line::styled("TERMINAL TOO SMALL", t.base().fg(t.warning)),
        Line::from(format!(
            "{} × {} detected; resize to at least 80 × 24.",
            area.width, area.height
        )),
        Line::from(if app.activity.is_some() {
            "A task is still running. Esc / q asks before cancelling."
        } else {
            "No test starts automatically. Navigation is preserved."
        }),
        Line::default(),
        Line::from("? help · Esc back · q quit · Ctrl+C stop"),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .style(t.base())
            .wrap(Wrap { trim: true }),
        area.inner(Margin::new(
            u16::from(area.width > 4),
            u16::from(area.height > 4),
        )),
    );
}

fn overlay(frame: &mut Frame, modal: Modal, t: Theme, area: Rect) {
    let (title, lines) = match modal {
        Modal::Help => (
            " KEYBOARD FIELD GUIDE ",
            vec![
                Line::styled("NAVIGATION", t.focus()),
                Line::from("↑/↓ or j/k     Move selection / scroll a report"),
                Line::from("Enter           Open, start or change a value"),
                Line::from("Tab / ←/→       Switch sibling sections"),
                Line::from("Shift+Tab        Previous section"),
                Line::from("Esc / Backspace  Back; confirm before cancelling"),
                Line::from("+ / - / Space   Edit a selected setting"),
                Line::from("PgUp / PgDn      Scroll report details"),
                Line::from("r               Reload history / retry a test"),
                Line::from("q               Quit; confirm if a task is running"),
                Line::from("Ctrl+C          Stop immediately (save finishes first)"),
                Line::default(),
                Line::styled("READABILITY", t.focus()),
                Line::from("Settings: adaptive colors and comfortable / compact layout."),
                Line::from("Larger body text: terminal Zoom In / profile font size."),
                Line::styled("Esc / ? / Enter  Close help", t.focus()),
            ],
        ),
        Modal::Cancel { quit, confirm } => (
            " CONFIRM CANCELLATION ",
            vec![
                Line::styled(
                    if quit {
                        "Cancel the active task and quit?"
                    } else {
                        "Cancel the active task?"
                    },
                    t.base().fg(t.warning).add_modifier(Modifier::BOLD),
                ),
                Line::default(),
                Line::from("No incomplete speed-test result will be saved."),
                Line::from("A running diagnostic command will be stopped."),
                Line::default(),
                Line::styled(
                    if confirm {
                        "  Continue task       › Yes, cancel"
                    } else {
                        "› Continue task         Yes, cancel"
                    },
                    t.focus(),
                ),
                Line::default(),
                Line::styled(
                    "←/→ select · Enter confirm · y cancel · Esc continue",
                    t.muted(),
                ),
            ],
        ),
    };
    let width = 64.min(area.width.saturating_sub(2)).max(1).min(area.width);
    let height = (lines.len() as u16 + 4)
        .min(area.height.saturating_sub(2))
        .max(1)
        .min(area.height);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .style(t.base())
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .title(title)
                    .title_style(t.focus())
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(t.base().fg(t.focus))
                    .padding(Padding::new(2, 2, 1, 1)),
            ),
        popup,
    );
}

fn speed(value: Option<f64>) -> String {
    value.map_or("—".into(), |v| format!("{v:.1} Mbps"))
}
fn ms(value: Option<f64>) -> String {
    value.map_or("n/a".into(), |v| format!("{v:.1} ms"))
}
fn grade_color(grade: QualityGrade, t: Theme) -> ratatui::style::Color {
    match grade {
        QualityGrade::APlus | QualityGrade::A | QualityGrade::B => t.success,
        QualityGrade::C | QualityGrade::D => t.warning,
        QualityGrade::F => t.error,
    }
}

fn severity_color(severity: FindingSeverity, t: Theme) -> ratatui::style::Color {
    match severity {
        FindingSeverity::Info => t.text,
        FindingSeverity::Warning => t.warning,
        FindingSeverity::Critical => t.error,
    }
}
