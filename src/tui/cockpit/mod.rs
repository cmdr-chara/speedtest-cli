//! Full-screen application shell. Owns navigation, not measurement business logic.
mod services;
mod state;
#[cfg(test)]
mod tests;
mod theme;
mod view;

use std::{
    future::pending,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossterm::event::{self, Event};
use futures_util::{future::BoxFuture, FutureExt};
use tokio::sync::mpsc;

use self::{
    services::Archive,
    state::{Activity, Cockpit, Effect},
    theme::Theme,
};
use super::{enter_terminal, frame_interval, restore_terminal, TerminalGuard, PHYSICS_RATE};
use crate::{engine::EngineEvent, model::TestResult, runtime, session::TestOptions};

enum Work {
    Measured(Result<Box<TestResult>, String>),
    Saved(Result<(), String>),
    Tool(Result<String, String>),
    History(Result<Box<Archive>, String>),
}
type Job = Option<BoxFuture<'static, Work>>;

async fn next_job(job: &mut Job) -> Work {
    match job {
        Some(future) => future.await,
        None => pending().await,
    }
}

async fn next_event(
    receiver: &mut Option<mpsc::UnboundedReceiver<EngineEvent>>,
) -> Option<EngineEvent> {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => pending().await,
    }
}

fn history_job() -> Job {
    Some(
        async {
            Work::History(
                tokio::task::spawn_blocking(Archive::load)
                    .await
                    .context("history reader stopped")
                    .and_then(|result| result)
                    .map(Box::new)
                    .map_err(|e| format!("{e:#}")),
            )
        }
        .boxed(),
    )
}

pub async fn run(options: TestOptions) -> Result<()> {
    let _guard = TerminalGuard;
    let mut terminal = enter_terminal()?;
    let mut app = Cockpit::new(options);
    let theme = Theme::detect();
    let mut history = history_job(); // Local reads only; no engine/client is constructed here.
    let mut job: Job = None;
    let mut receiver = None;
    let mut physics = tokio::time::interval(PHYSICS_RATE);
    let mut input = tokio::time::interval(Duration::from_millis(16));
    let mut fps = app.options.fps;
    let mut render = tokio::time::interval(frame_interval(fps));
    for interval in [&mut physics, &mut input, &mut render] {
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    }
    let mut dirty = true;
    let mut started = Instant::now();
    let mut heartbeat = 0;
    let mut size = terminal.size()?;
    let result = loop {
        let mut effect = Effect::None;
        tokio::select! {
            _ = input.tick() => {
                // Drain a bounded batch so repeated key/resize events cannot starve workers.
                for _ in 0..32 {
                    if !event::poll(Duration::ZERO).context("failed to poll cockpit input")? { break; }
                    match event::read().context("failed to read cockpit input")? {
                        Event::Key(key) => { effect = app.key_at_size(key, size.width, size.height); dirty = true; }
                        Event::Resize(width, height) => { size.width = width; size.height = height; dirty = true; },
                        _ => {}
                    }
                    if effect != Effect::None { break; }
                }
                let next = started.elapsed().as_millis() / 150;
                if app.activity.is_some() && next != heartbeat { heartbeat = next; dirty = true; }
            }
            _ = physics.tick(), if app.activity == Some(Activity::Test) && !app.reduced_motion => {
                dirty |= app.live.tick(PHYSICS_RATE);
            }
            _ = render.tick(), if dirty => {
                terminal.draw(|frame| view::draw(frame, &mut app, theme, started.elapsed()))
                    .context("failed to draw cockpit")?;
                dirty = false;
            }
            event = next_event(&mut receiver) => {
                match event { Some(event) => app.apply_engine(event), None => receiver = None }
                dirty = true;
            }
            work = next_job(&mut history) => {
                history = None;
                if let Work::History(result) = work { app.set_history(result.map(|archive| *archive)); }
                dirty = true;
            }
            work = next_job(&mut job) => {
                job = None;
                match work {
                    Work::Measured(result) => {
                        receiver = None;
                        app.measured(result.map(|r| *r));
                        if app.activity == Some(Activity::Saving) {
                            let options = app.options.clone();
                            let result = app.result.clone().expect("successful measurement retained");
                            // File writes are off the UI thread and are not cancelled mid-commit.
                            job = Some(async move {
                                Work::Saved(tokio::task::spawn_blocking(move || options.finish(&result)).await
                                    .context("result writer stopped").and_then(|r| r).map_err(|e| format!("{e:#}")))
                            }.boxed());
                        }
                    }
                    Work::Saved(result) => {
                        app.saved(result);
                        history = history_job();
                        if app.quit_after_save { break Ok(()); }
                    }
                    Work::Tool(result) => app.tool_finished(result),
                    Work::History(_) => unreachable!("history has its own job slot"),
                }
                dirty = true;
            }
        }
        match effect {
            Effect::StartTest => {
                let options = app.options.clone();
                let (tx, rx) = mpsc::unbounded_channel();
                receiver = Some(rx);
                started = Instant::now();
                job = Some(
                    async move {
                        let run = async {
                            let engine = options.engine()?;
                            runtime::deadline(Duration::from_secs(options.timeout), engine.run(tx))
                                .await
                        };
                        Work::Measured(
                            run.await
                                .map(Box::new)
                                .map_err(|e: anyhow::Error| format!("{e:#}")),
                        )
                    }
                    .boxed(),
                );
            }
            Effect::StartTool(tool) => {
                let options = app.options.clone();
                started = Instant::now();
                job = Some(
                    async move {
                        Work::Tool(
                            services::run_tool(tool, options)
                                .await
                                .map_err(|e| format!("{e:#}")),
                        )
                    }
                    .boxed(),
                );
            }
            Effect::LoadHistory => history = history_job(),
            Effect::Cancel => {
                job = None;
                receiver = None;
            }
            Effect::Quit => break Ok(()),
            Effect::Interrupt => break Err(runtime::Outcome::Cancelled.into()),
            Effect::None => {}
        }
        if fps != app.options.fps {
            fps = app.options.fps;
            render = tokio::time::interval(frame_interval(fps));
            render.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        }
    };
    // Drop the owned measurement/child process before restoring the terminal.
    drop(job);
    drop(history);
    restore_terminal(&mut terminal)?;
    result
}
