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

const TICK_RATE: Duration = Duration::from_millis(16);

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
        terminal
            .draw(|frame| view::draw(frame, &app))
            .context("failed to draw TUI")?;

        if let Some(error) = &app.error {
            return Err(anyhow!(error.clone()));
        }

        tokio::select! {
            _ = ticker.tick() => {
                app.tick(TICK_RATE);
                if let Some(result) = handle_input(&app)? {
                    return Ok(result);
                }
            }
            event = rx.recv(), if !app.is_complete() => {
                match event {
                    Some(event) => app.apply(event),
                    None => return Err(anyhow!("measurement engine stopped unexpectedly")),
                }
            }
        }
    }
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
