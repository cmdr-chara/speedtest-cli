mod app;
mod speedometer;
mod stability;
mod view;

use std::{
    io::{self, Stdout},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use crossterm::{
    cursor::Show,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    engine::EngineEvent,
    model::TestResult,
    stability::{StabilityEvent, StabilityResult},
};

use self::app::App;

const PHYSICS_RATE: Duration = Duration::from_nanos(4_166_667);

pub async fn run(mut rx: UnboundedReceiver<EngineEvent>, render_fps: u16) -> Result<TestResult> {
    let mut terminal = enter_terminal()?;
    let result = run_loop(&mut terminal, &mut rx, render_fps).await;
    let restoration = restore_terminal(&mut terminal);
    finish_terminal_session(result, restoration)
}

pub async fn run_stability(
    rx: UnboundedReceiver<StabilityEvent>,
    target_duration: Duration,
    render_fps: u16,
) -> Result<StabilityResult> {
    stability::run(rx, target_duration, render_fps).await
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    rx: &mut UnboundedReceiver<EngineEvent>,
    render_fps: u16,
) -> Result<TestResult> {
    let mut app = App::default();
    let mut physics = tokio::time::interval(PHYSICS_RATE);
    let mut render = tokio::time::interval(frame_interval(render_fps));
    physics.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    render.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut dirty = true;

    loop {
        if let Some(error) = &app.error {
            return Err(anyhow!(error.clone()));
        }

        tokio::select! {
            _ = physics.tick() => {
                if app.tick(PHYSICS_RATE) {
                    dirty = true;
                }
                if let Some(result) = handle_input(&app)? {
                    return Ok(result);
                }
            }
            _ = render.tick(), if dirty => {
                terminal
                    .draw(|frame| view::draw(frame, &app))
                    .context("failed to draw TUI")?;
                dirty = false;
            }
            event = rx.recv(), if !app.is_complete() => {
                match event {
                    Some(event) => {
                        app.apply(event);
                        dirty = true;
                    }
                    None => return Err(anyhow!("measurement engine stopped unexpectedly")),
                }
            }
        }
    }
}

pub(super) fn frame_interval(fps: u16) -> Duration {
    Duration::from_secs_f64(1.0 / f64::from(fps))
}

fn handle_input(app: &App) -> Result<Option<TestResult>> {
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
        return Err(anyhow!("speed test cancelled"));
    }

    Ok(None)
}

pub(super) fn enter_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("failed to enable raw terminal mode")?;
    let mut stdout = io::stdout();
    if let Err(error) = execute!(stdout, EnterAlternateScreen) {
        let setup_error = anyhow::Error::new(error).context("failed to enter alternate screen");
        return Err(setup_error_with_cleanup(
            setup_error,
            restore_stdout(&mut stdout),
        ));
    }

    match Terminal::new(CrosstermBackend::new(stdout)) {
        Ok(terminal) => Ok(terminal),
        Err(error) => {
            let setup_error = anyhow::Error::new(error).context("failed to initialize terminal");
            let mut stdout = io::stdout();
            Err(setup_error_with_cleanup(
                setup_error,
                restore_stdout(&mut stdout),
            ))
        }
    }
}

pub(super) fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let raw_mode = disable_raw_mode();
    let alternate_screen = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let cursor = terminal.show_cursor();
    terminal_restoration_result(raw_mode, alternate_screen, cursor)
}

fn restore_stdout(stdout: &mut Stdout) -> Result<()> {
    let raw_mode = disable_raw_mode();
    let alternate_screen = execute!(stdout, LeaveAlternateScreen);
    let cursor = execute!(stdout, Show);
    terminal_restoration_result(raw_mode, alternate_screen, cursor)
}

fn terminal_restoration_result(
    raw_mode: io::Result<()>,
    alternate_screen: io::Result<()>,
    cursor: io::Result<()>,
) -> Result<()> {
    let mut failures = Vec::new();
    for (step, result) in [
        ("disable raw mode", raw_mode),
        ("leave alternate screen", alternate_screen),
        ("restore cursor", cursor),
    ] {
        if let Err(error) = result {
            failures.push(format!("{step}: {error}"));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "failed to fully restore terminal: {}",
            failures.join("; ")
        ))
    }
}

fn setup_error_with_cleanup(setup_error: anyhow::Error, cleanup: Result<()>) -> anyhow::Error {
    match cleanup {
        Ok(()) => setup_error,
        Err(cleanup_error) => anyhow!(
            "terminal setup failed: {setup_error:#}; cleanup also failed: {cleanup_error:#}"
        ),
    }
}

pub(super) fn finish_terminal_session<T>(session: Result<T>, restoration: Result<()>) -> Result<T> {
    match (session, restoration) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(session_error), Err(restoration_error)) => Err(anyhow!(
            "terminal session failed: {session_error:#}; restoration also failed: {restoration_error:#}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_interval_matches_requested_cap() {
        assert_eq!(frame_interval(60).as_micros(), 16_666);
        assert_eq!(frame_interval(240).as_micros(), 4_166);
    }

    #[test]
    fn terminal_restoration_reports_all_failed_steps() {
        let error = terminal_restoration_result(
            Err(io::Error::other("raw failure")),
            Err(io::Error::other("screen failure")),
            Err(io::Error::other("cursor failure")),
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("disable raw mode: raw failure"));
        assert!(error.contains("leave alternate screen: screen failure"));
        assert!(error.contains("restore cursor: cursor failure"));
    }

    #[test]
    fn session_and_restoration_errors_are_both_retained() {
        let session = Err::<(), _>(anyhow!("draw failure"));
        let restoration = Err(anyhow!("cursor failure"));
        let error = finish_terminal_session(session, restoration)
            .unwrap_err()
            .to_string();

        assert!(error.contains("draw failure"));
        assert!(error.contains("cursor failure"));
    }
}
