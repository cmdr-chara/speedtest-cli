mod app;
mod speedometer;
mod view;

use std::{
    io::{self, Stdout},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{engine::EngineEvent, model::TestResult};

use self::app::App;

const PHYSICS_RATE: Duration = Duration::from_nanos(4_166_667);

pub async fn run(mut rx: UnboundedReceiver<EngineEvent>, render_fps: u16) -> Result<TestResult> {
    let mut terminal = enter_terminal()?;
    let result = run_loop(&mut terminal, &mut rx, render_fps).await;
    restore_terminal(&mut terminal)?;
    result
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

fn frame_interval(fps: u16) -> Duration {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_interval_matches_requested_cap() {
        assert_eq!(frame_interval(60).as_micros(), 16_666);
        assert_eq!(frame_interval(240).as_micros(), 4_166);
    }
}
