mod speedometer;

use std::{
    collections::VecDeque,
    io::{self, Stdout},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    prelude::{Alignment, Color, Constraint, Direction, Frame, Layout, Line, Modifier, Span, Style},
    widgets::{Block, Borders, Paragraph, Sparkline},
    Terminal,
};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    engine::EngineEvent,
    model::{TestPhase, TestResult},
};

const ACCENT: Color = Color::Cyan;
const TICK_RATE: Duration = Duration::from_millis(33);

#[derive(Debug)]
struct App {
    phase: TestPhase,
    current_mbps: f64,
    peak_mbps: f64,
    download_mbps: Option<f64>,
    upload_mbps: Option<f64>,
    ping_ms: Option<f64>,
    jitter_ms: Option<f64>,
    download_loaded_ms: Option<f64>,
    upload_loaded_ms: Option<f64>,
    samples: VecDeque<f64>,
    result: Option<TestResult>,
    error: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            phase: TestPhase::Preparing,
            current_mbps: 0.0,
            peak_mbps: 0.0,
            download_mbps: None,
            upload_mbps: None,
            ping_ms: None,
            jitter_ms: None,
            download_loaded_ms: None,
            upload_loaded_ms: None,
            samples: VecDeque::with_capacity(90),
            result: None,
            error: None,
        }
    }
}

impl App {
    fn apply(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::PhaseChanged(phase) => {
                if matches!(phase, TestPhase::Download | TestPhase::Upload) {
                    self.current_mbps = 0.0;
                    self.peak_mbps = 0.0;
                    self.samples.clear();
                }
                self.phase = phase;
            }
            EngineEvent::IdleLatency { ping_ms, jitter_ms } => {
                self.ping_ms = Some(ping_ms);
                self.jitter_ms = Some(jitter_ms);
            }
            EngineEvent::ThroughputSample { phase, mbps } => {
                self.phase = phase;
                self.current_mbps = mbps;
                self.peak_mbps = self.peak_mbps.max(mbps);
                if self.samples.len() == 90 {
                    self.samples.pop_front();
                }
                self.samples.push_back(mbps);
            }
            EngineEvent::LoadedLatency { phase, ms } => match phase {
                TestPhase::Download => self.download_loaded_ms = Some(ms),
                TestPhase::Upload => self.upload_loaded_ms = Some(ms),
                _ => {}
            },
            EngineEvent::Complete(result) => {
                self.download_mbps = Some(result.download.mbps);
                self.upload_mbps = Some(result.upload.mbps);
                self.download_loaded_ms = result.latency.download_loaded_ms;
                self.upload_loaded_ms = result.latency.upload_loaded_ms;
                self.current_mbps = result.download.mbps;
                self.peak_mbps = self
                    .peak_mbps
                    .max(result.download.mbps)
                    .max(result.upload.mbps);
                self.phase = TestPhase::Complete;
                self.result = Some(result);
            }
            EngineEvent::Error(error) => self.error = Some(error),
        }
    }
}

pub async fn run(mut rx: UnboundedReceiver<EngineEvent>) -> Result<TestResult> {
    let mut terminal = enter_terminal()?;
    let result = run_loop(&mut terminal, &mut rx).await;
    restore_terminal(&mut terminal)?;
    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    rx: &mut UnboundedReceiver<EngineEvent>,
) -> Result<TestResult> {
    let mut app = App::default();
    let mut ticker = tokio::time::interval(TICK_RATE);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        terminal.draw(|frame| draw(frame, &app)).context("failed to draw TUI")?;

        if let Some(error) = &app.error {
            return Err(anyhow!(error.clone()));
        }

        if let Some(result) = &app.result {
            tokio::time::sleep(Duration::from_millis(450)).await;
            return Ok(result.clone());
        }

        tokio::select! {
            _ = ticker.tick() => {
                if event::poll(Duration::ZERO).context("failed to poll terminal input")? {
                    if let Event::Key(key) = event::read().context("failed to read terminal input")? {
                        if key.kind == KeyEventKind::Press
                            && (key.code == KeyCode::Char('q')
                                || key.code == KeyCode::Esc
                                || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)))
                        {
                            return Err(anyhow!("speed test cancelled"));
                        }
                    }
                }
            }
            event = rx.recv() => {
                match event {
                    Some(event) => app.apply(event),
                    None => return Err(anyhow!("measurement engine stopped unexpectedly")),
                }
            }
        }
    }
}

fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let outer = Block::default()
        .title(Line::from(" SPEEDTEST ").style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(9),
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(2),
        ])
        .split(inner);

    let value = if matches!(app.phase, TestPhase::Download | TestPhase::Upload | TestPhase::Complete) {
        format!("{:.1} Mbps", app.current_mbps)
    } else if let Some(ping) = app.ping_ms {
        format!("{ping:.1} ms")
    } else {
        "—".to_string()
    };

    let headline = Paragraph::new(vec![
        Line::from(Span::styled(
            value,
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            app.phase.label(),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )),
    ])
    .alignment(Alignment::Center);
    frame.render_widget(headline, vertical[0]);

    let scale = speedometer::scale_for(app.peak_mbps.max(app.current_mbps));
    speedometer::render(frame, vertical[1], app.current_mbps, scale, ACCENT);

    let metrics = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(vertical[2]);

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

    let data: Vec<u64> = app.samples.iter().map(|value| value.max(0.0) as u64).collect();
    let sparkline = Sparkline::default()
        .block(Block::default().borders(Borders::TOP).border_style(Style::default().fg(Color::DarkGray)))
        .data(&data)
        .style(Style::default().fg(ACCENT));
    frame.render_widget(sparkline, vertical[3]);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Cloudflare Edge", Style::default().fg(Color::Gray)),
        Span::raw("  •  "),
        Span::styled("q / esc / ctrl-c to cancel", Style::default().fg(Color::DarkGray)),
    ]))
    .alignment(Alignment::Center);
    frame.render_widget(footer, vertical[4]);
}

fn metric_line(label: &'static str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {label:<10}"), Style::default().fg(Color::DarkGray)),
        Span::styled(value, Style::default().fg(Color::White)),
    ])
}

fn format_speed(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_string(), |value| format!("{value:.1} Mbps"))
}

fn format_ms(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_string(), |value| format!("{value:.1} ms"))
}

fn enter_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("failed to enable raw terminal mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
    Terminal::new(CrosstermBackend::new(stdout)).context("failed to initialize terminal")
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().context("failed to disable raw terminal mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to restore cursor")?;
    Ok(())
}
