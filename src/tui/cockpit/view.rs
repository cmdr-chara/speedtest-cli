use crate::i18n::ui;
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
    let _locale = crate::i18n::scope(app.language);
    let area = frame.area();
    frame.render_widget(Block::default().style(theme.base()), area);
    if area.width < 80 || area.height < 24 {
        small(frame, app, theme, area);
    } else {
        let inner = area.inner(Margin::new(2, 1));
        // Short summaries and the live dial keep their controls nearby;
        // scrolling reports retain the full available viewport.
        let content = match app.screen() {
            Screen::Home => {
                Constraint::Length(inner.height.saturating_sub(6).min(if app.compact {
                    14
                } else {
                    24
                }))
            }
            Screen::Live => Constraint::Length(inner.height.saturating_sub(6).min(35)),
            _ => Constraint::Min(1),
        };
        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(2),
            content,
            Constraint::Length(2),
        ])
        .flex(ratatui::layout::Flex::Start)
        .split(inner);
        chrome(frame, app, theme, rows[0], rows[1]);
        let mut footer_area = rows[3];
        match app.screen() {
            Screen::Home => home(frame, app, theme, rows[2]),
            Screen::Configure | Screen::Settings => configure(frame, app, theme, rows[2]),
            Screen::Live => live(frame, app, theme, rows[2], elapsed),
            Screen::Results => {
                let used = results(frame, app, theme, rows[2]);
                footer_area.y = rows[2].y + used;
            }
            Screen::History => {
                let used = history(frame, app, theme, rows[2]);
                footer_area.y = rows[2].y + used;
            }
            Screen::Statistics => {
                let used = statistics(frame, app, theme, rows[2]);
                footer_area.y = rows[2].y + used;
            }
            Screen::Compare => {
                let used = compare(frame, app, theme, rows[2]);
                footer_area.y = rows[2].y + used;
            }
            Screen::Dns | Screen::Diagnostics => tools(frame, app, theme, rows[2]),
            Screen::Tool => report(frame, app, theme, rows[2], elapsed),
            Screen::Failure => failure(frame, app, theme, rows[2]),
        }
        footer(frame, app, theme, footer_area);
    }
    if let Some(modal) = app.modal {
        overlay(frame, app, modal, theme, area);
    }
}

fn metric_height(app: &Cockpit, area: Rect) -> u16 {
    if !app.compact && area.width >= 90 && area.height >= 19 {
        8
    } else {
        3
    }
}

/// Highlight the label, never its description or trailing whitespace.
fn choice(label: &str, selected: bool, t: Theme) -> Line<'static> {
    let label = ui(label);
    Line::from(vec![
        Span::styled(ui(if selected { "› " } else { "  " }), t.focus()),
        Span::styled(
            ui(format!(" {label} ")),
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
                Span::styled(ui("SPEEDTEST"), t.focus()),
                Span::styled(ui(" / NETWORK COCKPIT"), t.muted()),
            ]),
            Line::styled(
                app.pages
                    .iter()
                    .map(|page| ui(page.screen.title()))
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
        None => ("", t.muted),
    };
    frame.render_widget(
        Paragraph::new(ui(status))
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
            let label = ui(label);
            Line::from(ui(if i == index {
                format!("> {label}")
            } else {
                label.to_string()
            }))
        })
        .collect();
    let mut start = 0;
    while start < index
        && titles[start..=index]
            .iter()
            .map(|line| line.width() + 2)
            .sum::<usize>()
            > usize::from(tabs.width)
    {
        start += 1;
    }
    frame.render_widget(
        Tabs::new(titles.into_iter().skip(start).collect::<Vec<_>>())
            .select(index - start)
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
            Line::styled(ui(title), t.focus()),
            Line::styled(ui(single(subtitle)), t.muted()),
        ])
        .wrap(Wrap { trim: true }),
        rows[0],
    );
    rows[1]
}

fn home(frame: &mut Frame, app: &Cockpit, t: Theme, area: Rect) {
    let rows = Layout::vertical([Constraint::Length(5), Constraint::Min(1)]).split(area);
    let hero = Layout::horizontal([
        Constraint::Length((u32::from(area.width) * 55 / 100).min(48) as u16),
        Constraint::Min(1),
    ])
    .split(rows[0]);
    let mut logo: Vec<_> = BRAND.iter().map(|s| Line::styled(*s, t.focus())).collect();
    logo.push(Line::styled(ui("Your network, in focus."), t.strong()));
    frame.render_widget(Paragraph::new(logo), hero[0]);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(ui("MEASUREMENT PROFILE"), t.strong()),
            Line::styled(app.options.backend_label(), t.focus()),
            Line::styled(
                ui(format!(
                    "{} s / phase  ·  {} streams",
                    app.options.duration, app.options.streams
                )),
                t.muted(),
            ),
            Line::styled(ui("No background network probes"), t.muted()),
        ])
        .wrap(Wrap { trim: true }),
        hero[1],
    );
    let spacious = !app.compact && rows[1].height >= 18;
    let columns = Layout::horizontal([
        Constraint::Length(if area.width >= 140 {
            44
        } else if area.width >= 96 {
            34
        } else {
            29
        }),
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
                Paragraph::new(ui(*description))
                    .style(t.muted())
                    .wrap(Wrap { trim: true }),
                Rect::new(
                    columns[0].x + 3,
                    y + 1,
                    columns[0].width.saturating_sub(5),
                    2,
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
        let metric_rows = if spacious { 8 } else { 3 };
        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(metric_rows),
            Constraint::Min(1),
        ])
        .split(detail);
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(ui("LATEST RESULT"), t.strong()),
                Line::styled(
                    ui(format!(
                        "{} UTC · {}",
                        result.timestamp.format("%d %b %H:%M"),
                        single(&result.backend)
                    )),
                    t.muted(),
                ),
            ]),
            rows[0],
        );
        let metric_area = Rect::new(rows[1].x, rows[1].y, rows[1].width.min(80), rows[1].height);
        let metrics = Layout::horizontal([Constraint::Percentage(50); 2]).split(metric_area);
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
        let mut lines = vec![Line::from(ui(format!(
            "Idle {:.1} ms · jitter {:.1} ms",
            result.latency.idle_ms, result.latency.jitter_ms
        )))];
        if let Some(analysis) = &result.analysis {
            lines.push(Line::styled(
                ui(format!(
                    "Quality {}/100 · {} · {} confidence",
                    analysis.quality.score,
                    analysis.quality.grade.label(),
                    analysis.quality.confidence.label()
                )),
                t.strong().fg(grade_color(analysis.quality.grade, t)),
            ));
            if spacious {
                lines.push(Line::default());
                if let Some(finding) = analysis.quality.findings.first() {
                    lines.push(Line::styled(
                        ui(single(&finding.title)),
                        t.strong().fg(severity_color(finding.severity, t)),
                    ));
                    lines.push(Line::from(ui(single(&finding.evidence))));
                } else {
                    lines.push(Line::from(ui(
                        "No findings in this result. This is not a continuous connection monitor.",
                    )));
                }
            }
        }
        let summary = Paragraph::new(lines)
            .style(t.base())
            .wrap(Wrap { trim: true });
        let summary_height = summary
            .line_count(rows[2].width)
            .min(usize::from(rows[2].height.saturating_sub(2))) as u16;
        frame.render_widget(
            summary,
            Rect::new(rows[2].x, rows[2].y, rows[2].width, summary_height),
        );
        if rows[2].height > 0 {
            // A wrapped finding may exceed this preview; keep its full-result
            // action visible even when the window has no more room to grow.
            let link_y = rows[2].y + (summary_height + 1).min(rows[2].height - 1);
            frame.render_widget(
                Paragraph::new(Line::styled(ui("v  Open result"), t.focus())),
                Rect::new(rows[2].x, link_y, rows[2].width, 1),
            );
        }
    } else {
        let (title, text, color) = match &app.history {
            Load::Loading => ("READING LOCAL HISTORY", "Loading saved results. No network activity.", t.text),
            Load::Failed(_) => ("HISTORY UNAVAILABLE", "Local history could not be read. Open History for details, or press r to retry. You can still run a test.", t.warning),
            Load::Ready(_) => ("No tests yet", "Start your first test to see throughput, latency and connection quality here.\n\nYour connection has not been probed.", t.text),
        };
        let mut lines = vec![
            Line::styled(ui("RECENT ACTIVITY"), t.strong()),
            Line::default(),
            Line::styled(ui(title), t.strong().fg(color)),
            Line::default(),
        ];
        lines.extend(ui(text).lines().map(|s| Line::from(s.to_owned())));
        frame.render_widget(
            Paragraph::new(lines)
                .style(t.base())
                .wrap(Wrap { trim: true }),
            detail,
        );
    }
}

fn metric(frame: &mut Frame, label: &str, value: &str, unit: &str, t: Theme, area: Rect) {
    let digit_height = if area.height >= 7 { 5 } else { 3 };
    frame.render_widget(
        Paragraph::new(ui(label)).style(t.strong()),
        Rect::new(area.x, area.y, area.width, 1),
    );
    if area.height >= 5
        && numerals::draw(
            frame,
            Rect::new(
                area.x,
                area.y + 1,
                area.width.saturating_sub(1),
                digit_height,
            ),
            value,
            t.strong(),
            Alignment::Left,
        )
    {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(value.to_owned(), t.strong()),
                Span::styled(ui(format!(" {unit}")), t.muted()),
            ])),
            Rect::new(area.x, area.y + digit_height + 1, area.width, 1),
        );
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(value.to_owned(), t.strong()),
                Span::styled(ui(format!(" {unit}")), t.muted()),
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
    fields.push(("Language", app.language.label().into()));
    let spacing = u16::from(!app.compact && columns[0].height >= (fields.len() * 2) as u16);
    let rows: Vec<_> = fields
        .iter()
        .enumerate()
        .map(|(index, (label, value))| {
            let selected = index == app.page().selected;
            Row::new(vec![
                Cell::from(choice(label, selected, t)),
                Cell::from(ui(value.clone())).style(if selected { t.focus() } else { t.strong() }),
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
        Some(10) => ("Language", "Change the interface language immediately. Commands, flags, JSON and CSV remain unchanged. Preferences apply to this session."),
        _ => ("READABILITY", "Comfortable uses large metric digits and extra row spacing where space allows. Compact uses ordinary text.\n\nFor larger body text, use your terminal's Zoom In or profile font-size setting. The app reflows after resizing and never changes your terminal font."),
    };
    let mut lines = vec![Line::styled(ui(title), t.focus()), Line::default()];
    lines.extend(
        ui(description)
            .lines()
            .map(|line| Line::from(line.to_owned())),
    );
    lines.extend([
        Line::default(),
        Line::styled(ui("Enter / + / -  Change selected value"), t.strong()),
        Line::default(),
    ]);
    if app.options.output.is_some() {
        lines.push(Line::styled(
            ui("Explicit export enabled"),
            t.base().fg(t.warning),
        ));
        lines.push(Line::from(ui(
            "--output is written after each test, even with history off. The same path is reused.",
        )));
    } else {
        lines.push(Line::styled(
            ui(if app.options.no_save {
                "History OFF · results stay in this session"
            } else {
                "History ON · completed tests are saved locally"
            }),
            t.muted(),
        ));
    }
    if app.options.librespeed_server.is_some() {
        lines.push(Line::from(ui(
            "Custom LibreSpeed endpoint configured; used when LibreSpeed is selected.",
        )));
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
            ui(format!(
                "{} {}{}  ",
                index + 1,
                if active { "› " } else { "" },
                ui(step.label())
            )),
            if active { t.focus() } else { t.muted() },
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(rail)), rows[0]);
    let columns = Layout::horizontal([
        Constraint::Length((u32::from(rows[1].width) * 64 / 100).min(96) as u16),
        Constraint::Length(3),
        Constraint::Min(1),
    ])
    .split(rows[1]);
    let gauge = columns[0];
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
            ui(if app.activity == Some(Activity::Saving) {
                "FINISHING"
            } else {
                phase.label()
            }),
            t.focus(),
        ),
        Line::styled(
            ui(format!("Elapsed {}s · latest samples", elapsed.as_secs())),
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
            Line::from(ui(format!("Down   {}", speed(app.live.download_mbps)))),
            Line::from(ui(format!("Up     {}", speed(app.live.upload_mbps)))),
            Line::default(),
        ]);
        side
    };
    lines.extend([
        Line::from(ui(format!("Idle   {}", ms(app.live.ping_ms)))),
        Line::from(ui(format!("Jitter {}", ms(app.live.jitter_ms)))),
        Line::from(ui(format!("Load ↓ {}", ms(app.live.download_loaded_ms)))),
        Line::from(ui(format!("Load ↑ {}", ms(app.live.upload_loaded_ms)))),
    ]);
    frame.render_widget(Paragraph::new(lines).style(t.base()), details);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(ui(format!(
                    "{} · {} streams · {}s per transfer phase",
                    app.options.backend_label(),
                    app.options.streams,
                    app.options.duration
                )),
                t.muted(),
            ),
            Line::styled(ui("Gauge is smoothed; samples are provisional. Final results use completed measurements."),
                t.muted(),
            ),
        ]),
        rows[2],
    );
}

/// Return the used body height so the footer follows short results while long
/// findings retain the full available scroll viewport.
fn results(frame: &mut Frame, app: &mut Cockpit, t: Theme, area: Rect) -> u16 {
    let top = area.y;
    let Some(result) = app.result.as_ref() else {
        return area.height;
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
    let roomy = height >= 6 && area.width >= 100 && rows[1].height >= 12;
    let mut summary = vec![
        Line::styled(
            ui(if roomy && app.save_notice.starts_with("SAVE FAILED") {
                "SAVE FAILED · See details →".into()
            } else {
                single(&app.save_notice)
            }),
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
            Line::styled(ui("LOADED LATENCY"), t.strong()),
            Line::from(ui(format!(
                "Download  {}",
                ms(result.latency.download_loaded_ms)
            ))),
            Line::from(ui(format!(
                "Upload    {}",
                ms(result.latency.upload_loaded_ms)
            ))),
            Line::default(),
        ]);
    } else {
        summary.push(Line::from(ui(format!(
            "Loaded latency   Download {}   /   Upload {}",
            ms(result.latency.download_loaded_ms),
            ms(result.latency.upload_loaded_ms)
        ))));
    }
    let mut findings = Vec::new();
    if roomy && app.save_notice.starts_with("SAVE FAILED") {
        findings.extend([
            Line::styled(ui("SAVE FAILED"), t.strong().fg(t.error)),
            Line::from(ui(single(&app.save_notice))),
            Line::default(),
        ]);
    }
    if let Some(analysis) = &result.analysis {
        let q = &analysis.quality;
        summary.push(Line::styled(
            format!(
                "{}  {}/100  {}{}",
                ui("QUALITY"),
                q.score,
                q.grade.label(),
                if q.is_s_tier() { " / S-TIER" } else { "" }
            ),
            t.strong().fg(grade_color(q.grade, t)),
        ));
        summary.push(Line::from(ui(format!(
            "{} confidence · bufferbloat {}",
            q.confidence.label(),
            q.bufferbloat.grade.map_or("n/a", QualityGrade::label)
        ))));
        summary.push(Line::from(ui(format!(
            "Idle p95 {:.1} / p99 {:.1} ms",
            analysis.latency.idle.p95_ms, analysis.latency.idle.p99_ms
        ))));
        if roomy {
            summary.extend([
                Line::default(),
                Line::styled(ui("APPLICATION READINESS"), t.strong()),
                Line::from(ui(format!("Gaming        {}", q.workloads.gaming.label()))),
                Line::from(ui(format!(
                    "Video calls   {}",
                    q.workloads.video_calls.label()
                ))),
                Line::from(ui(format!(
                    "Streaming     {}",
                    q.workloads.streaming.label()
                ))),
                Line::from(ui(format!(
                    "Cloud gaming  {}",
                    q.workloads.cloud_gaming.label()
                ))),
            ]);
        } else {
            summary.push(Line::from(ui(format!(
                "Gaming {}   Calls {}   Streaming {}   Cloud gaming {}",
                q.workloads.gaming.label(),
                q.workloads.video_calls.label(),
                q.workloads.streaming.label(),
                q.workloads.cloud_gaming.label()
            ))));
        }
        if q.findings.is_empty() {
            findings.extend([
                Line::styled(ui("NO FINDINGS"), t.strong().fg(t.success)),
                Line::from(ui(
                    "No issues were flagged by the local analysis of this result.",
                )),
            ]);
        }
        for finding in &q.findings {
            findings.push(Line::styled(
                format!(
                    "{} · {}",
                    ui(finding.severity.label()),
                    ui(single(&finding.title))
                ),
                t.strong().fg(severity_color(finding.severity, t)),
            ));
            findings.push(Line::from(ui(single(&finding.evidence))));
            if let Some(recommendation) = &finding.recommendation {
                findings.push(Line::default());
                findings.push(Line::from(ui(single(recommendation))));
            }
            findings.push(Line::default());
        }
    } else {
        findings.push(Line::from(ui(
            "Quality analysis unavailable for this saved result.",
        )));
    }
    findings.push(Line::styled(
        ui("Mbps = decimal megabits/s. HTTP latency is not ICMP; scores are local heuristics."),
        t.muted(),
    ));
    if roomy {
        let columns = Layout::horizontal([
            Constraint::Length(if area.width >= 140 { 44 } else { 34 }),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(rows[1]);
        let summary = Paragraph::new(summary)
            .style(t.base())
            .wrap(Wrap { trim: true });
        let findings_width = columns[2].width.min(100);
        let findings = Paragraph::new(findings)
            .style(t.base())
            .wrap(Wrap { trim: false });
        let findings_height = findings.line_count(findings_width);
        let content_height = summary
            .line_count(columns[0].width)
            .max(findings_height)
            .min(usize::from(rows[1].height)) as u16;
        frame.render_widget(
            summary,
            Rect::new(columns[0].x, columns[0].y, columns[0].width, content_height),
        );
        frame.render_widget(
            Block::default()
                .borders(Borders::LEFT)
                .border_style(t.base().fg(t.line)),
            Rect::new(columns[1].x, columns[1].y, columns[1].width, content_height),
        );
        scroll_paragraph(
            frame,
            app,
            t,
            Rect::new(columns[2].x, columns[2].y, findings_width, content_height),
            findings,
        );
        rows[1].y - top + content_height.saturating_add(1).min(rows[1].height)
    } else {
        summary.push(Line::default());
        summary.extend(findings);
        let content_height = scroll(frame, app, t, rows[1], summary);
        rows[1].y - top + content_height.saturating_add(1).min(rows[1].height)
    }
}

fn result_metrics(frame: &mut Frame, result: &TestResult, t: Theme, area: Rect) {
    let area = Rect::new(area.x, area.y, area.width.min(160), area.height);
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
        let mut lines = vec![Line::styled(ui(title), t.focus()), Line::default()];
        lines.extend(ui(message).lines().map(|line| Line::from(line.to_owned())));
        scroll(frame, app, t, area, lines);
        true
    } else {
        false
    }
}

fn history(frame: &mut Frame, app: &mut Cockpit, t: Theme, area: Rect) -> u16 {
    let top = area.y;
    let available = area.height;
    let area = heading(
        frame,
        "YOUR NETWORK, OVER TIME",
        "Last 30 days · newest first · Enter opens a result",
        t,
        area,
    );
    if archive_state(frame, app, t, area) {
        return available;
    }
    let Load::Ready(archive) = &app.history else {
        return available;
    };
    let preview = !app.compact && area.height >= 19;
    let preview_height = if preview { 9 } else { 0 };
    let table_height = archive
        .results
        .len()
        .saturating_add(2)
        .min(usize::from(area.height.saturating_sub(2 + preview_height)))
        as u16;
    let layout = Layout::vertical([
        Constraint::Length(table_height),
        Constraint::Length(2),
        Constraint::Length(preview_height),
    ])
    .flex(ratatui::layout::Flex::Start)
    .split(area);
    app.history_page_size = usize::from(layout[0].height.saturating_sub(2).max(1));
    let rows: Vec<_> = archive
        .results
        .iter()
        .rev()
        .map(|r| {
            let quality = r.analysis.as_ref().map_or("n/a".into(), |a| {
                format!("{} {}", a.quality.score, a.quality.grade.label())
            });
            Row::new(vec![
                Cell::from(if app.is_baseline(r) { "B" } else { "" }),
                Cell::from(r.timestamp.format("%m-%d %H:%M").to_string()),
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
                Constraint::Length(1),
                Constraint::Length(if area.width >= 100 { 15 } else { 11 }),
                Constraint::Length(if area.width >= 100 { 13 } else { 11 }),
                Constraint::Fill(1),
                Constraint::Fill(1),
                Constraint::Fill(1),
                Constraint::Length(9),
            ],
        )
        .header(
            Row::new(vec![
                Cell::from(""),
                Cell::from(ui("DATE / UTC")),
                Cell::from(ui("BACKEND")),
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
        Rect::new(
            layout[0].x,
            layout[0].y,
            layout[0].width.min(120),
            layout[0].height,
        ),
        &mut app.table,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                ui(format!(
                    "Run {} of {} · PgUp/PgDn page · Home/End",
                    app.page().selected + 1,
                    archive.results.len()
                )),
                t.muted(),
            ),
            Line::styled(
                app.baseline.as_ref().map_or_else(
                    || ui("b pin baseline · c compare selected with its older neighbor"),
                    |baseline| {
                        ui(format!(
                            "BASELINE {} UTC · {}",
                            baseline.timestamp.format("%m-%d %H:%M:%S"),
                            single(&baseline.backend)
                        ))
                    },
                ),
                t.focus(),
            ),
        ]),
        layout[1],
    );
    if preview {
        if let Some(selected) = archive.newest(app.page().selected) {
            let preview_area = Rect::new(
                layout[2].x,
                layout[2].y,
                layout[2].width.min(160),
                layout[2].height,
            );
            let block = Block::default()
                .borders(Borders::TOP)
                .border_style(t.base().fg(t.line))
                .title(ui(" SELECTED RESULT "))
                .title_style(t.strong());
            let detail = block.inner(preview_area);
            frame.render_widget(block, preview_area);
            let parts =
                Layout::vertical([Constraint::Length(2), Constraint::Length(6)]).split(detail);
            frame.render_widget(
                Paragraph::new(ui(format!(
                    "{} UTC · {} · Enter opens details",
                    selected.timestamp.format("%d %b %Y %H:%M"),
                    single(&selected.backend)
                )))
                .style(t.muted()),
                parts[0],
            );
            result_metrics(frame, selected, t, parts[1]);
        }
    }
    layout[2]
        .bottom()
        .saturating_sub(top)
        .saturating_add(1)
        .min(available)
}

fn right(value: String) -> Cell<'static> {
    Cell::from(Line::from(ui(value)).alignment(Alignment::Right))
}

fn statistics(frame: &mut Frame, app: &mut Cockpit, t: Theme, area: Rect) -> u16 {
    let top = area.y;
    let available = area.height;
    let area = heading(
        frame,
        "THE BIGGER PICTURE",
        "30-day local history · Enter compares the latest two saved runs",
        t,
        area,
    );
    if archive_state(frame, app, t, area) {
        return available;
    }
    let Load::Ready(archive) = &app.history else {
        return available;
    };
    let Some(summary) = &archive.summary else {
        return available;
    };
    let rows = Layout::vertical([
        Constraint::Length(metric_height(app, area)),
        Constraint::Min(1),
    ])
    .split(area);
    let columns = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(Rect::new(
        rows[0].x,
        rows[0].y,
        rows[0].width.min(120),
        rows[0].height,
    ));
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
                        .title(ui("Download · saved samples, not continuous monitoring"))
                        .title_style(t.strong()),
                )
                .x_axis(
                    Axis::default()
                        .bounds([0.0, (data.len() - 1) as f64])
                        .labels(vec![Line::from(ui("oldest")), Line::from(ui("latest"))])
                        .style(t.base().fg(t.line)),
                )
                .y_axis(
                    Axis::default()
                        .bounds([0.0, maximum])
                        .labels(vec![
                            Line::from(ui("0")),
                            Line::from(ui(format!("{maximum:.1} Mbps"))),
                        ])
                        .style(t.base().fg(t.line)),
                ),
                parts[0],
            );
        } else {
            frame.render_widget(
                Paragraph::new(ui(
                    "ONE SAVED SAMPLE\n\nSave another test to see a download history chart.",
                ))
                .style(t.base()),
                parts[0],
            );
        }
    }
    let mut lines = vec![
        Line::styled(
            ui(format!(
                "{} RUNS  ·  TREND: {}",
                summary.runs,
                summary.trend.label().to_uppercase()
            )),
            t.focus(),
        ),
        Line::styled(
            ui(if plot {
                String::new()
            } else {
                summary.download_sparkline.clone()
            }),
            t.focus(),
        ),
        Line::from(ui(format!(
            "Best down {:.1} Mbps  /  best up {:.1} Mbps  /  p95 idle {:.1} ms",
            summary.best_download_mbps, summary.best_upload_mbps, summary.p95_ping_ms
        ))),
        Line::from(ui(format!(
            "Median quality {}  /  S-tier runs {}",
            summary
                .median_quality_score
                .map_or("n/a".into(), |n| format!("{n:.0}/100")),
            summary.s_tier_runs
        ))),
        Line::default(),
    ];
    if summary.anomalies.is_empty() {
        lines.push(Line::styled(
            ui("No anomaly flags in this sample. This is not a guarantee of stability."),
            t.muted(),
        ));
    }
    for anomaly in &summary.anomalies {
        lines.push(Line::styled(
            format!(
                "{} · {}",
                ui(anomaly.severity.label()),
                ui(single(&anomaly.message))
            ),
            t.base().fg(t.warning),
        ));
    }
    let used = scroll(frame, app, t, parts[1], lines);
    parts[1].y - top + used.saturating_add(1).min(parts[1].height)
}

fn compare(frame: &mut Frame, app: &mut Cockpit, t: Theme, area: Rect) -> u16 {
    let top = area.y;
    let available = area.height;
    let area = heading(
        frame,
        "BEFORE / AFTER",
        "Saved result snapshots · no new measurement · Home/End jumps through details",
        t,
        area,
    );
    let comparison = app.comparison.as_ref().or(match &app.history {
        Load::Ready(archive) => archive.comparison.as_ref(),
        _ => None,
    });
    let Some(comparison) = comparison else {
        if archive_state(frame, app, t, area) {
            return available;
        }
        frame.render_widget(Paragraph::new(ui("Two saved tests are needed for a comparison.\n\nRun another test with history enabled, then press r to reload.")).style(t.base()).wrap(Wrap { trim: true }), area);
        return available;
    };
    let c = &comparison.metrics;
    let mut lines = vec![];
    for (label, result) in [("BEFORE", &comparison.before), ("AFTER", &comparison.after)] {
        lines.push(Line::styled(
            ui(format!(
                "{label} {} UTC · {}",
                result.timestamp.format("%Y-%m-%d %H:%M:%S%.f"),
                single(&result.backend)
            )),
            t.strong(),
        ));
    }
    lines.push(Line::default());
    let mut rows = vec![];
    for (label, delta, unit) in [
        ("Download", &c.download_mbps, "Mbps"),
        ("Upload", &c.upload_mbps, "Mbps"),
        ("Idle latency", &c.ping_ms, "ms"),
        ("Jitter", &c.jitter_ms, "ms"),
    ] {
        rows.push(ComparisonRow::new(
            label,
            format!("{:.1} {unit}", delta.before),
            format!("{:.1} {unit}", delta.after),
            Some(delta.absolute_change),
            Some(delta.improved),
            unit,
        ));
    }
    for (label, delta, unit) in [
        ("Quality", &c.quality_score, "pts"),
        ("Loaded increase", &c.bufferbloat_ms, "ms"),
    ] {
        let value = |value: Option<f64>| {
            value.map_or_else(
                || ui("n/a"),
                |value| {
                    if unit == "pts" {
                        format!("{value:.0}/100")
                    } else {
                        format!("{value:.1} {unit}")
                    }
                },
            )
        };
        rows.push(ComparisonRow::new(
            label,
            value(delta.before),
            value(delta.after),
            delta.absolute_change,
            delta.improved,
            unit,
        ));
    }
    lines.extend(comparison_table(rows, t, area.width));
    lines.extend([
        Line::styled(ui(single(&c.verdict)), t.focus()),
        Line::from(ui(single(&c.highlight))),
        Line::default(),
        Line::from(ui(format!(
            "Before server: {}",
            single(&comparison.before.server.host)
        ))),
        Line::from(ui(format!(
            "After server: {}",
            single(&comparison.after.server.host)
        ))),
        Line::default(),
        Line::styled(
            ui("Loaded increase is worst bufferbloat; n/a means the saved run lacks that analysis."),
            t.muted(),
        ),
        Line::styled(
            ui("Paths, backends and test conditions can differ; compare like-for-like runs."),
            t.muted(),
        ),
    ]);
    let used = scroll(frame, app, t, area, lines);
    area.y - top + used.saturating_add(1).min(area.height)
}

struct ComparisonRow {
    cells: [String; 4],
    improved: Option<bool>,
}

impl ComparisonRow {
    fn new(
        label: &str,
        before: String,
        after: String,
        change: Option<f64>,
        improved: Option<bool>,
        unit: &str,
    ) -> Self {
        let direction = match (change, improved) {
            (Some(change), _) if change.abs() < f64::EPSILON => "same",
            (_, Some(true)) => "better",
            (_, Some(false)) => "worse",
            _ => "n/a",
        };
        let delta = change.map_or_else(
            || ui("n/a"),
            |change| {
                let value = if unit == "pts" {
                    format!("{change:+.0}")
                } else {
                    format!("{change:+.1}")
                };
                format!("{value} {unit} ({})", ui(direction))
            },
        );
        Self {
            cells: [ui(label), before, after, delta],
            improved: improved.filter(|_| direction != "same"),
        }
    }
}

/// Measure terminal cells after translation. When the complete values cannot fit
/// side by side, stack labeled values rather than clipping evidence or units.
fn comparison_table(rows: Vec<ComparisonRow>, t: Theme, width: u16) -> Vec<Line<'static>> {
    let headers = [ui("METRIC"), ui("BEFORE"), ui("AFTER"), ui("CHANGE")];
    let mut widths = headers
        .each_ref()
        .map(|value| Line::from(value.as_str()).width());
    for row in &rows {
        for (index, cell) in row.cells.iter().enumerate() {
            widths[index] = widths[index].max(Line::from(cell.as_str()).width());
        }
    }
    let fits = widths.iter().sum::<usize>() + 6 <= usize::from(width);
    let aligned = |cells: [String; 4], style| {
        let mut spans = vec![];
        for (index, cell) in cells.into_iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw("  "));
            }
            let pad = " ".repeat(widths[index].saturating_sub(Line::from(cell.as_str()).width()));
            spans.push(Span::styled(
                if index == 0 {
                    format!("{cell}{pad}")
                } else {
                    format!("{pad}{cell}")
                },
                style,
            ));
        }
        Line::from(spans)
    };
    let mut lines = vec![];
    if fits {
        lines.push(aligned(headers.clone(), t.strong()));
    }
    for row in rows {
        let style = t.base().fg(match row.improved {
            Some(true) => t.success,
            Some(false) => t.warning,
            None => t.text,
        });
        if fits {
            lines.push(aligned(row.cells, style));
        } else {
            lines.push(Line::styled(row.cells[0].clone(), t.strong()));
            for (index, cell) in row.cells.into_iter().enumerate().skip(1) {
                lines.push(Line::styled(format!("{}: {cell}", headers[index]), style));
            }
            lines.push(Line::default());
        }
    }
    lines
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
            ui(if selected.network() {
                "NETWORK CHECK · MANUAL START"
            } else {
                "LOCAL INSPECTION"
            }),
            t.focus(),
        ),
        Line::default(),
        Line::from(selected.description()),
        Line::default(),
        Line::styled(ui("Enter  Open tool"), t.focus()),
        Line::default(),
    ];
    if dns {
        lines.push(Line::styled(ui("DNS changes stay in the CLI: set, optimize, reset and rollback retain their confirmation/rollback flow."), t.muted()));
    } else {
        lines.push(Line::styled(
            ui("LAN server/client and advanced command options remain available from the CLI."),
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
            Line::styled(ui("READY TO START"), t.focus()), Line::default(), Line::from(tool.description()), Line::default(),
            choice("Enter  Start diagnostic", true, t), Line::default(),
            Line::styled(ui(format!("Overall deadline: {} seconds. Settings apply to this session.", app.options.timeout)), t.muted()),
        ],
        Some(Load::Loading) => {
            let pulse = if app.reduced_motion { "●" } else { ["◐", "◓", "◑", "◒"][(elapsed.as_millis() / 150 % 4) as usize] };
            vec![Line::styled(ui(format!("{pulse} RUNNING  ·  {}s elapsed", elapsed.as_secs())), t.focus()), Line::default(),
                Line::from(tool.description()), Line::default(), Line::styled(ui("The interface stays responsive. Esc cancels with confirmation."), t.muted())]
        }
        Some(Load::Failed(error)) => vec![Line::styled(ui("DIAGNOSTIC FAILED"), t.base().fg(t.error).add_modifier(Modifier::BOLD)),
            Line::default(), Line::from(single(error)), Line::default(),
            Line::from(ui("Check connectivity, permissions or availability of native tools. No configuration was changed.")),
            Line::default(), Line::styled(ui("Enter / r  Retry    Esc  Back"), t.focus())],
        Some(Load::Ready(text)) => {
            let mut lines = vec![Line::styled(ui("COMPLETED · Enter / r runs again"), t.base().fg(t.success)), Line::default()];
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
    let lines = vec![Line::styled(ui("ERROR"), t.base().fg(t.error).add_modifier(Modifier::BOLD)), Line::default(),
        Line::from(single(&app.failure)), Line::default(),
        Line::from(ui("Check connectivity, return to Test to change backend, or increase the deadline in Settings.")),
        Line::default(), Line::styled(ui("Enter / r  Retry    Esc  Configuration    Tab  Another section"), t.focus())];
    scroll(frame, app, t, area, lines);
}

fn scroll(
    frame: &mut Frame,
    app: &mut Cockpit,
    t: Theme,
    area: Rect,
    lines: Vec<Line<'static>>,
) -> u16 {
    let paragraph = Paragraph::new(Text::from(lines))
        .style(t.base())
        .wrap(Wrap { trim: false });
    scroll_paragraph(frame, app, t, area, paragraph)
}

/// Draw a report and return its occupied height, including an overflow hint.
fn scroll_paragraph(
    frame: &mut Frame,
    app: &mut Cockpit,
    t: Theme,
    area: Rect,
    paragraph: Paragraph<'static>,
) -> u16 {
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
            Paragraph::new(ui(format!(
                "↑↓ {}–{} / {} · PgUp/PgDn",
                usize::from(app.page().scroll) + 1,
                (usize::from(app.page().scroll) + usize::from(visible)).min(count),
                count
            )))
            .style(t.muted()),
            Rect::new(area.x, area.y + visible, area.width, 1),
        );
    }
    count.min(usize::from(area.height)) as u16
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
            Screen::History => "Enter result  ·  b pin baseline  ·  c compare  ·  r reload".into(),
            Screen::Statistics => "Enter compare (from Stats)  ·  j/k scroll  ·  r reload".into(),
            Screen::Compare => {
                "j/k or PgUp/PgDn scroll  ·  Home/End  ·  Esc back to saved runs".into()
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
            Span::styled(ui("?"), t.focus()),
            Span::styled(ui(" help  ·  "), t.muted()),
            Span::styled(ui("q"), t.focus()),
            Span::styled(ui(" quit  ·  "), t.muted()),
            Span::styled(ui("Navigation resumes when the task finishes."), t.muted()),
        ])
    } else {
        Line::from(vec![
            Span::styled(ui("j/k"), t.focus()),
            Span::styled(ui(" move  ·  "), t.muted()),
            Span::styled(ui("Tab / ←→"), t.focus()),
            Span::styled(ui(" sections  ·  "), t.muted()),
            Span::styled(ui("Esc"), t.focus()),
            Span::styled(ui(" back  ·  "), t.muted()),
            Span::styled(ui("?"), t.focus()),
            Span::styled(ui(" help  ·  "), t.muted()),
            Span::styled(ui("q"), t.focus()),
            Span::styled(ui(" quit"), t.muted()),
        ])
    };
    frame.render_widget(
        Paragraph::new(vec![Line::styled(ui(context), t.muted()), bindings]),
        area,
    );
}

fn small(frame: &mut Frame, app: &Cockpit, t: Theme, area: Rect) {
    let text = vec![
        Line::styled(ui("SPEEDTEST / NETWORK COCKPIT"), t.focus()),
        Line::default(),
        Line::styled(ui("TERMINAL TOO SMALL"), t.base().fg(t.warning)),
        Line::from(ui(format!(
            "{} × {} detected; resize to at least 80 × 24.",
            area.width, area.height
        ))),
        Line::from(ui(if app.activity.is_some() {
            "A task is still running. Esc / q asks before cancelling."
        } else {
            "No test starts automatically. Navigation is preserved."
        })),
        Line::default(),
        Line::from(ui("? help · Esc back · q quit · Ctrl+C stop")),
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

fn overlay(frame: &mut Frame, app: &mut Cockpit, modal: Modal, t: Theme, area: Rect) {
    let (title, lines) = match modal {
        Modal::TextSize => (
            " TEXT SIZE ",
            vec![
                Line::styled(ui("LARGER TEXT, NOT JUST LARGER PANELS"), t.focus()),
                Line::from(ui("Your terminal controls the size of every character. Use its Zoom In action or increase the profile font size.")),
                Line::from(ui("Keep at least 80 columns and 24 rows. The cockpit reflows when you zoom or resize. Comfortable layout enlarges metrics only.")),
                Line::default(),
                Line::styled(ui("ARCH / OMARCHY"), t.focus()),
                Line::from(ui("Omarchy defaults to Alacritty, but other terminals are supported. For Alacritty, edit the existing [font] size in:")),
                Line::from(ui("~/.config/alacritty/alacritty.toml")),
                Line::from("[font]"),
                Line::from("size = 14.0"),
                Line::default(),
                Line::styled(ui("OTHER TERMINALS"), t.focus()),
                Line::from(ui("Ghostty: font-size = 14 in its config. macOS Terminal and Windows Terminal: increase font size in profile settings.")),
                Line::from(ui("Keep your other settings and imports. Do not add a duplicate [font] section.")),
                Line::default(),
                Line::from(ui("No terminal, desktop, font or configuration setting is changed by this guide.")),
                Line::from(ui("")),
                Line::default(),
                Line::styled(ui("↑↓ scroll · Esc / Enter close"), t.focus()),
            ],
        ),
        Modal::Help => (
            " KEYBOARD FIELD GUIDE ",
            vec![
                Line::styled(ui("NAVIGATION"), t.focus()),
                Line::from(ui("↑/↓ or j/k     Move selection / scroll a report")),
                Line::from(ui("Enter           Open, start or change a value")),
                Line::from(ui("Tab / ←/→       Switch sibling sections")),
                Line::from(ui("Shift+Tab        Previous section")),
                Line::from(ui("Esc / Backspace  Back; confirm before cancelling")),
                Line::from(ui("+ / - / Space   Edit a selected setting")),
                Line::from(ui("PgUp / PgDn      Scroll report details")),
                Line::from(ui("History: PgUp/PgDn page; Home/End first/last run.")),
                Line::from(ui("History: b pin baseline; c compare selected with baseline or older neighbor.")),
                Line::from(ui("r               Reload history / retry a test")),
                Line::from(ui("q               Quit; confirm if a task is running")),
                Line::from(ui("Ctrl+C          Stop immediately (save finishes first)")),
                Line::default(),
                Line::styled(ui("READABILITY"), t.focus()),
                Line::from(ui("z  Text size guide; terminal font controls body text.")),
                Line::from(ui("Settings: adaptive colors and comfortable / compact layout.")),
                Line::from(ui("Your terminal controls the size of every character. Use its Zoom In action or increase the profile font size.")),
                Line::styled(ui("Esc / ? / Enter  Close help"), t.focus()),
            ],
        ),
        Modal::Cancel { quit, confirm } => (
            " CONFIRM CANCELLATION ",
            vec![
                Line::styled(ui(if quit {
                        "Cancel the active task and quit?"
                    } else {
                        "Cancel the active task?"
                    }),
                    t.base().fg(t.warning).add_modifier(Modifier::BOLD),
                ),
                Line::default(),
                Line::from(ui("No incomplete speed-test result will be saved.")),
                Line::from(ui("A running diagnostic command will be stopped.")),
                Line::default(),
                Line::styled(ui(if confirm {
                        "  Continue task       › Yes, cancel"
                    } else {
                        "› Continue task         Yes, cancel"
                    }),
                    t.focus(),
                ),
                Line::default(),
                Line::styled(ui("←/→ select · Enter confirm · y cancel · Esc continue"),
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
    let block = Block::default()
        .title(ui(title))
        .title_style(t.focus())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(t.base().fg(t.focus))
        .padding(Padding::new(2, 2, 1, 1));
    let inner = block.inner(popup);
    let paragraph = Paragraph::new(lines)
        .style(t.base())
        .wrap(Wrap { trim: true });
    app.modal_scroll = app.modal_scroll.min(
        paragraph
            .line_count(inner.width)
            .saturating_sub(usize::from(inner.height))
            .min(usize::from(u16::MAX)) as u16,
    );
    frame.render_widget(paragraph.scroll((app.modal_scroll, 0)).block(block), popup);
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
