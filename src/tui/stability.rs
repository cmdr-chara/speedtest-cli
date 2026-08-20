use std::{collections::VecDeque, time::Duration};

use anyhow::{anyhow, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    backend::CrosstermBackend,
    prelude::{
        Alignment, Color, Constraint, Direction, Frame, Layout, Line, Modifier, Rect, Span, Style,
    },
    widgets::{Block, Borders, Gauge, Paragraph, Sparkline},
    Terminal,
};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    analysis,
    model::QualityGrade,
    stability::{StabilityEvent, StabilityResult, StabilitySample},
};

use super::{enter_terminal, frame_interval, restore_terminal};

const TRACE_POINTS: usize = 120;

#[derive(Debug)]
struct StabilityApp {
    target_duration: Duration,
    elapsed_ms: u64,
    current_ms: Option<f64>,
    recent: VecDeque<f64>,
    successful: Vec<f64>,
    probes: usize,
    failed: usize,
    result: Option<StabilityResult>,
}

impl StabilityApp {
    fn new(target_duration: Duration) -> Self {
        Self {
            target_duration,
            elapsed_ms: 0,
            current_ms: None,
            recent: VecDeque::with_capacity(TRACE_POINTS),
            successful: Vec::new(),
            probes: 0,
            failed: 0,
            result: None,
        }
    }

    fn apply(&mut self, event: StabilityEvent) {
        match event {
            StabilityEvent::Sample(sample) => self.apply_sample(sample),
            StabilityEvent::Complete(result) => {
                self.elapsed_ms = self.target_duration.as_millis().min(u128::from(u64::MAX)) as u64;
                self.result = Some(*result);
            }
        }
    }

    fn apply_sample(&mut self, sample: StabilitySample) {
        self.elapsed_ms = sample.elapsed_ms;
        self.probes += 1;
        match sample.latency_ms {
            Some(latency) => {
                self.current_ms = Some(latency);
                self.successful.push(latency);
                if self.recent.len() == TRACE_POINTS {
                    self.recent.pop_front();
                }
                self.recent.push_back(latency);
            }
            None => self.failed += 1,
        }
    }

    fn is_complete(&self) -> bool {
        self.result.is_some()
    }
}

pub(super) async fn run(
    mut rx: UnboundedReceiver<StabilityEvent>,
    target_duration: Duration,
    render_fps: u16,
) -> Result<StabilityResult> {
    let mut terminal = enter_terminal()?;
    let result = run_loop(&mut terminal, &mut rx, target_duration, render_fps).await;
    restore_terminal(&mut terminal)?;
    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    rx: &mut UnboundedReceiver<StabilityEvent>,
    target_duration: Duration,
    render_fps: u16,
) -> Result<StabilityResult> {
    let mut app = StabilityApp::new(target_duration);
    let mut input = tokio::time::interval(Duration::from_millis(33));
    let mut render = tokio::time::interval(frame_interval(render_fps));
    input.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    render.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut dirty = true;

    loop {
        tokio::select! {
            _ = input.tick() => {
                if let Some(result) = handle_input(&app)? {
                    return Ok(result);
                }
            }
            _ = render.tick(), if dirty => {
                terminal
                    .draw(|frame| draw(frame, &app))
                    .context("failed to draw stability TUI")?;
                dirty = false;
            }
            event = rx.recv(), if !app.is_complete() => {
                match event {
                    Some(event) => {
                        app.apply(event);
                        dirty = true;
                    }
                    None => return Err(anyhow!("stability probe task stopped unexpectedly")),
                }
            }
        }
    }
}

fn handle_input(app: &StabilityApp) -> Result<Option<StabilityResult>> {
    if !event::poll(Duration::ZERO).context("failed to poll terminal input")? {
        return Ok(None);
    }
    let Event::Key(key) = event::read().context("failed to read terminal input")? else {
        return Ok(None);
    };
    if key.kind != KeyEventKind::Press {
        return Ok(None);
    }

    if app.is_complete() && matches!(key.code, KeyCode::Enter | KeyCode::Char('q') | KeyCode::Esc) {
        return Ok(app.result.clone());
    }
    if key.code == KeyCode::Char('q')
        || key.code == KeyCode::Esc
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
    {
        return Err(anyhow!("stability test cancelled"));
    }
    Ok(None)
}

fn draw(frame: &mut Frame, app: &StabilityApp) {
    let area = frame.area();
    let outer = Block::default()
        .title(
            Line::from(" NETWORK STABILITY ").style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(7),
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

    render_progress(frame, app, layout[0]);
    render_trace(frame, app, layout[1]);
    render_metrics(frame, app, layout[2]);
    render_status(frame, app, layout[3]);
    render_footer(frame, app, layout[4]);
}

fn render_progress(frame: &mut Frame, app: &StabilityApp, area: Rect) {
    let target_ms = app.target_duration.as_millis().max(1) as f64;
    let progress = (app.elapsed_ms as f64 / target_ms).clamp(0.0, 1.0);
    let label = if app.is_complete() {
        format!("complete  •  {}", format_clock(app.elapsed_ms))
    } else {
        format!(
            "{} / {}  •  {:.0}%",
            format_clock(app.elapsed_ms),
            format_clock(app.target_duration.as_millis() as u64),
            progress * 100.0
        )
    };
    let gauge = Gauge::default()
        .ratio(progress)
        .label(label)
        .gauge_style(Style::default().fg(Color::Cyan))
        .use_unicode(true);
    frame.render_widget(gauge, area);
}

fn render_trace(frame: &mut Frame, app: &StabilityApp, area: Rect) {
    let data: Vec<u64> = app
        .recent
        .iter()
        .map(|latency| latency.max(0.0).round() as u64)
        .collect();
    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .title(" LATENCY TRACE ")
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .data(&data)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(sparkline, area);
}

fn render_metrics(frame: &mut Frame, app: &StabilityApp, area: Rect) {
    let stats = analysis::distribution(&app.successful);
    let halves = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let left = Paragraph::new(vec![
        metric_line("CURRENT", format_ms(app.current_ms)),
        metric_line(
            "MEDIAN",
            stats
                .as_ref()
                .map(|stats| stats.median_ms)
                .map(format_ms_value)
                .unwrap_or_else(|| "—".to_string()),
        ),
        metric_line(
            "P95",
            stats
                .as_ref()
                .map(|stats| stats.p95_ms)
                .map(format_ms_value)
                .unwrap_or_else(|| "—".to_string()),
        ),
    ]);
    let right = Paragraph::new(vec![
        metric_line(
            "P99",
            stats
                .as_ref()
                .map(|stats| stats.p99_ms)
                .map(format_ms_value)
                .unwrap_or_else(|| "—".to_string()),
        ),
        metric_line(
            "MAX",
            stats
                .as_ref()
                .map(|stats| stats.max_ms)
                .map(format_ms_value)
                .unwrap_or_else(|| "—".to_string()),
        ),
        metric_line(
            "PROBES",
            format!(
                "{} ok / {} failed",
                app.probes.saturating_sub(app.failed),
                app.failed
            ),
        ),
    ]);
    frame.render_widget(left, halves[0]);
    frame.render_widget(right, halves[1]);
}

fn render_status(frame: &mut Frame, app: &StabilityApp, area: Rect) {
    let paragraph = if let Some(result) = &app.result {
        let color = if result.s_tier {
            Color::LightCyan
        } else {
            grade_color(result.grade)
        };
        let tier = result
            .tier_label()
            .map_or(String::new(), |tier| format!("  ◆ {tier}"));
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!(
                    "STABILITY {}/100 {}{tier}",
                    result.score,
                    result.grade.label()
                ),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )),
            Line::from(format!(
                "probe availability {:.2}%  •  {} failed  •  {} failure bursts",
                result.probe_availability_percent, result.failed_probes, result.failure_bursts
            )),
        ])
        .alignment(Alignment::Center)
    } else {
        let availability = if app.probes == 0 {
            100.0
        } else {
            app.probes.saturating_sub(app.failed) as f64 / app.probes as f64 * 100.0
        };
        Paragraph::new(vec![
            Line::from(Span::styled(
                "LIVE STABILITY PROBE",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(format!("probe availability {availability:.2}%")),
        ])
        .alignment(Alignment::Center)
    };
    frame.render_widget(paragraph, area);
}

fn render_footer(frame: &mut Frame, app: &StabilityApp, area: Rect) {
    let instruction = if app.is_complete() {
        "enter / q / esc to close"
    } else {
        "q / esc / ctrl-c to cancel"
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Cloudflare Edge", Style::default().fg(Color::Gray)),
            Span::raw("  •  "),
            Span::styled(instruction, Style::default().fg(Color::DarkGray)),
        ]))
        .alignment(Alignment::Center),
        area,
    );
}

fn metric_line(label: &'static str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {label:<9}"), Style::default().fg(Color::DarkGray)),
        Span::styled(value, Style::default().fg(Color::White)),
    ])
}

fn grade_color(grade: QualityGrade) -> Color {
    match grade {
        QualityGrade::APlus | QualityGrade::A => Color::Green,
        QualityGrade::B => Color::Cyan,
        QualityGrade::C | QualityGrade::D => Color::Yellow,
        QualityGrade::F => Color::Red,
    }
}

fn format_ms(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_string(), format_ms_value)
}

fn format_ms_value(value: f64) -> String {
    format!("{value:.1} ms")
}

fn format_clock(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}
