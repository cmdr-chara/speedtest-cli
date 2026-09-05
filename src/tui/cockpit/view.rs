use std::time::Duration;

use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::Modifier,
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, Padding, Paragraph, Row, Table, Tabs, Wrap,
    },
    Frame,
};

use super::{
    services::{Tool, HISTORY_DAYS},
    state::{Activity, Cockpit, Load, Modal, Screen, SECTIONS},
    theme::Theme,
};
use crate::{
    model::{QualityGrade, TestPhase, TestResult},
    output,
    tui::speedometer,
};

const BRAND: [&str; 3] = [
    "┏━┓┏━┓┏━╸┏━╸╺┳┓╺┳╸┏━╸┏━┓╺┳╸",
    "┗━┓┣━┛┣╸ ┣╸  ┃┃ ┃ ┣╸ ┗━┓ ┃ ",
    "┗━┛╹  ┗━╸┗━╸╺┻┛ ╹ ┗━╸┗━┛ ╹ ",
];

pub(super) fn draw(frame: &mut Frame, app: &mut Cockpit, theme: Theme, elapsed: Duration) {
    let area = frame.area();
    frame.render_widget(Block::default().style(theme.base()), area);
    if area.width < 80 || area.height < 24 {
        small(frame, app, theme, area);
    } else {
        let inner = area.inner(Margin::new(2, 1));
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
        Layout::horizontal([Constraint::Percentage(57), Constraint::Percentage(43)]).split(rows[0]);
    let mut logo: Vec<_> = BRAND.iter().map(|s| Line::styled(*s, t.focus())).collect();
    logo.push(Line::styled("Your network, in focus.", t.muted()));
    frame.render_widget(Paragraph::new(logo), hero[0]);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled("MEASUREMENT PROFILE", t.muted()),
            Line::styled(
                app.options.backend_label(),
                t.base().add_modifier(Modifier::BOLD),
            ),
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
    let columns =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).split(rows[1]);
    let labels = [
        "Run Speed Test",
        "History",
        "Statistics",
        "DNS Tools",
        "Diagnostics",
        "Settings",
    ];
    let mut y = columns[0].y;
    for (index, label) in labels.iter().enumerate() {
        let selected = app.page().selected == index;
        let style = if selected { t.selected() } else { t.base() };
        let height = if index == 0 { 3 } else { 1 };
        let rect = Rect::new(columns[0].x, y, columns[0].width.saturating_sub(2), height);
        let mut lines = vec![Line::styled(
            format!("{} {label}", if selected { "›" } else { " " }),
            style,
        )];
        if index == 0 {
            lines.push(Line::styled(
                "  Configure, then start",
                if selected { style } else { t.muted() },
            ));
        }
        frame.render_widget(Paragraph::new(lines).style(style), rect);
        y += height;
    }
    let divider = Block::default()
        .borders(Borders::LEFT)
        .border_style(t.base().fg(t.line))
        .padding(Padding::new(3, 0, 0, 0));
    let detail = divider.inner(columns[1]);
    frame.render_widget(divider, columns[1]);
    if let Some(result) = app.latest() {
        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(4),
            Constraint::Min(1),
        ])
        .split(detail);
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled("LATEST RESULT", t.muted()),
                Line::styled(
                    format!(
                        "{} UTC · {}",
                        result.timestamp.format("%d %b %H:%M"),
                        single(&result.backend)
                    ),
                    t.base(),
                ),
            ]),
            rows[0],
        );
        let metrics = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[1]);
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
                t.base().fg(grade_color(analysis.quality.grade, t)),
            ));
        }
        lines.push(Line::styled("v  Open result", t.muted()));
        frame.render_widget(
            Paragraph::new(lines)
                .style(t.base())
                .wrap(Wrap { trim: true }),
            rows[2],
        );
    } else {
        let (title, text, color) = match &app.history {
            Load::Loading => ("READING LOCAL HISTORY", "Loading saved results. No network activity.", t.muted),
            Load::Failed(_) => ("HISTORY UNAVAILABLE", "Local history could not be read. Open History for details, or press r to retry. You can still run a test.", t.warning),
            Load::Ready(_) => ("No tests yet", "Start your first test to see throughput, latency and connection quality here.\n\nYour connection has not been probed.", t.text),
        };
        let mut lines = vec![
            Line::styled("RECENT ACTIVITY", t.muted()),
            Line::default(),
            Line::styled(title, t.base().fg(color).add_modifier(Modifier::BOLD)),
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
        Paragraph::new(vec![
            Line::styled(label.to_owned(), t.muted()),
            Line::from(vec![
                Span::styled(value.to_owned(), t.focus()),
                Span::styled(format!(" {unit}"), t.muted()),
            ]),
        ]),
        area,
    );
}

fn configure(frame: &mut Frame, app: &Cockpit, t: Theme, area: Rect) {
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
    ]);
    let rows: Vec<_> = fields
        .iter()
        .enumerate()
        .map(|(index, (label, value))| {
            let style = if index == app.page().selected {
                t.selected()
            } else {
                t.base()
            };
            Row::new(vec![
                format!(
                    "{} {label}",
                    if index == app.page().selected {
                        "›"
                    } else {
                        " "
                    }
                ),
                value.clone(),
            ])
            .style(style)
        })
        .collect();
    frame.render_widget(
        Table::new(
            rows,
            [Constraint::Percentage(56), Constraint::Percentage(44)],
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
    let mut lines = vec![
        Line::styled("BEFORE YOU START", t.focus()),
        Line::default(),
        Line::from("This test saturates your connection and can use substantial data."),
        Line::default(),
        Line::from("Idle latency → download → upload → quality analysis."),
        Line::default(),
    ];
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
    frame.render_widget(
        Paragraph::new(lines)
            .style(t.base())
            .wrap(Wrap { trim: true }),
        info,
    );
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
    let columns =
        Layout::horizontal([Constraint::Percentage(67), Constraint::Percentage(33)]).split(rows[1]);
    speedometer::render_with_background(
        frame,
        columns[0],
        &app.live.speedometer,
        t.focus,
        matches!(
            phase,
            TestPhase::Download | TestPhase::Upload | TestPhase::Complete
        ),
        t.background,
    );
    let (label, value) = if app.activity == Some(Activity::Saving) {
        ("FINISHING", "Saving completed result".into())
    } else {
        (phase.label(), format!("Elapsed {}s", elapsed.as_secs()))
    };
    let lines = vec![
        Line::styled(label, t.focus()),
        Line::styled(value, t.muted()),
        Line::default(),
        Line::from(format!("Down   {}", speed(app.live.download_mbps))),
        Line::from(format!("Up     {}", speed(app.live.upload_mbps))),
        Line::default(),
        Line::from(format!("Idle   {}", ms(app.live.ping_ms))),
        Line::from(format!("Jitter {}", ms(app.live.jitter_ms))),
        Line::from(format!("Load ↓ {}", ms(app.live.download_loaded_ms))),
        Line::from(format!("Load ↑ {}", ms(app.live.upload_loaded_ms))),
    ];
    frame.render_widget(Paragraph::new(lines).style(t.base()), columns[1]);
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
                "Live values are provisional. Final results use completed measurements.",
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
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(area);
    result_metrics(frame, result, t, rows[0]);
    let mut lines = vec![
        Line::styled(
            single(&app.save_notice),
            t.base().fg(if app.save_notice.starts_with("SAVE FAILED") {
                t.error
            } else {
                t.muted
            }),
        ),
        Line::default(),
        Line::from(format!(
            "Loaded latency   Download {}   /   Upload {}",
            ms(result.latency.download_loaded_ms),
            ms(result.latency.upload_loaded_ms)
        )),
    ];
    if let Some(analysis) = &result.analysis {
        let q = &analysis.quality;
        lines.push(Line::styled(
            format!(
                "QUALITY  {}/100  {}{}  ·  {} confidence",
                q.score,
                q.grade.label(),
                if q.is_s_tier() { " / S-TIER" } else { "" },
                q.confidence.label()
            ),
            t.base()
                .fg(grade_color(q.grade, t))
                .add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::from(format!(
            "Bufferbloat {}   ·   idle p95 {:.1} ms / p99 {:.1} ms",
            q.bufferbloat
                .grade
                .map_or("not measured", QualityGrade::label),
            analysis.latency.idle.p95_ms,
            analysis.latency.idle.p99_ms
        )));
        lines.push(Line::from(format!(
            "Gaming {}   Calls {}   Streaming {}   Cloud gaming {}",
            q.workloads.gaming.label(),
            q.workloads.video_calls.label(),
            q.workloads.streaming.label(),
            q.workloads.cloud_gaming.label()
        )));
        for finding in &q.findings {
            lines.push(Line::default());
            lines.push(Line::styled(
                format!("{} · {}", finding.severity.label(), single(&finding.title)),
                t.focus(),
            ));
            lines.push(Line::from(single(&finding.evidence)));
            if let Some(recommendation) = &finding.recommendation {
                lines.push(Line::from(single(recommendation)));
            }
        }
    } else {
        lines.push(Line::from(
            "Quality analysis unavailable for this saved result.",
        ));
    }
    lines.extend([
        Line::default(),
        Line::styled(
            "Mbps = decimal megabits/s. HTTP latency is not ICMP; scores are local heuristics.",
            t.muted(),
        ),
    ]);
    scroll(frame, app, t, rows[1], lines);
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
                Constraint::Length(15),
                Constraint::Length(11),
                Constraint::Length(12),
                Constraint::Length(12),
                Constraint::Length(9),
                Constraint::Min(7),
            ],
        )
        .header(
            Row::new([
                "DATE / UTC",
                "BACKEND",
                "DOWN Mbps",
                "UP Mbps",
                "IDLE ms",
                "QUALITY",
            ])
            .style(t.muted())
            .bottom_margin(1),
        )
        .row_highlight_style(t.selected())
        .highlight_symbol("› ")
        .column_spacing(1)
        .style(t.base()),
        area,
        &mut app.table,
    );
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
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(area);
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
    let mut lines = vec![
        Line::styled(
            format!(
                "{} RUNS  ·  TREND: {}",
                summary.runs,
                summary.trend.label().to_uppercase()
            ),
            t.focus(),
        ),
        Line::styled(summary.download_sparkline.clone(), t.focus()),
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
    scroll(frame, app, t, rows[1], lines);
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

fn tools(frame: &mut Frame, app: &Cockpit, t: Theme, area: Rect) {
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
            let style = if i == app.page().selected {
                t.selected()
            } else {
                t.base()
            };
            [
                Line::styled(
                    format!(
                        "{} {}",
                        if i == app.page().selected { "›" } else { " " },
                        tool.title()
                    ),
                    style,
                ),
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
    frame.render_widget(
        Paragraph::new(lines)
            .style(t.base())
            .wrap(Wrap { trim: true }),
        detail,
    );
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
            Line::styled("Enter  Start diagnostic", t.selected()), Line::default(),
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
    // Ratatui's line count includes wrapping, so resize cannot strand the viewport.
    let max = paragraph
        .line_count(area.width)
        .saturating_sub(usize::from(area.height))
        .min(usize::from(u16::MAX)) as u16;
    app.page_mut().scroll = app.page().scroll.min(max);
    frame.render_widget(paragraph.scroll((app.page().scroll, 0)), area);
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
            Screen::Configure | Screen::Settings => {
                "Enter start / edit  ·  +/- change value  ·  settings are session-only".into()
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
                Line::styled(
                    "Network checks start only by your explicit action.",
                    t.muted(),
                ),
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
