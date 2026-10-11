//! Opt-in visual evidence from the actual renderer. No clocks, terminals or WAN probes.
use std::{path::Path, time::Duration};

use clap::Parser;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, style::Modifier, Terminal};

use super::{
    services::Archive,
    state::{Activity, Cockpit, Screen},
    theme::{Palette, Theme},
    view,
};
use crate::{
    cli::Cli,
    engine::EngineEvent,
    model::{TestPhase, TestResult},
    session::TestOptions,
};

const WIDTH: u16 = 120;
const HEIGHT: u16 = 38;
const FRAME_MS: u64 = 50;

// Keep this fixture identical to capture_readability_frames_when_explicitly_requested
// so the before/after stills compare presentation, not different result data.
fn fixture() -> (Cockpit, TestResult) {
    let mut app = Cockpit::new(TestOptions::from(&Cli::parse_from([
        "speedtest",
        "--no-save",
    ])));
    app.language = crate::i18n::Language::En;
    app.palette = Palette::Graphite;
    let mut latest: TestResult =
        serde_json::from_str(include_str!("../../../tests/fixtures/result.json")).unwrap();
    latest.timestamp = chrono::DateTime::parse_from_rfc3339("2026-09-05T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    latest.download.mbps = 455.5;
    latest.upload.mbps = 312.3;
    latest.download.bytes = 56_937_500;
    latest.upload.bytes = 39_037_500;
    latest.analysis = Some(crate::analysis::build_network_analysis(
        &[8.0, 10.0, 12.0],
        &[18.0, 20.0, 22.0],
        &[28.0, 30.0, 32.0],
        &latest.latency,
        &latest.download,
        &latest.upload,
    ));
    let samples = [512.0, 630.0, 605.0, 680.0, 580.0, 720.0, 694.0, 455.5]
        .into_iter()
        .enumerate()
        .map(|(index, mbps)| {
            let mut sample = latest.clone();
            sample.timestamp -= chrono::Duration::hours(7 - index as i64);
            sample.download.mbps = mbps;
            sample
        })
        .collect();
    app.set_history(Ok(Archive::from_results(samples)));
    app.result = Some(latest.clone());
    app.save_notice = "SAVED to local history · example data".into();
    (app, latest)
}

fn press(app: &mut Cockpit, key: KeyCode) {
    app.key(KeyEvent::new(key, KeyModifiers::NONE));
}

fn capture(root: &Path, name: &str, app: &mut Cockpit, now: Duration, elapsed: Duration) {
    app.sync_motion(now, WIDTH, HEIGHT);
    app.motion.advance(now);
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, app, Theme::rgb(), elapsed))
        .unwrap();
    app.motion.finish_frame();
    let buffer = terminal.backend().buffer();
    let cells: Vec<_> = buffer
        .content
        .iter()
        .map(|cell| {
            serde_json::json!({
                "symbol": cell.symbol(),
                "fg": format!("{:?}", cell.fg),
                "bg": format!("{:?}", cell.bg),
                "bold": cell.modifier.contains(Modifier::BOLD),
                "dim": cell.modifier.contains(Modifier::DIM),
                "italic": cell.modifier.contains(Modifier::ITALIC),
                "underlined": cell.modifier.contains(Modifier::UNDERLINED),
                "reversed": cell.modifier.contains(Modifier::REVERSED),
            })
        })
        .collect();
    let path = root.join(format!("{name}.json"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        serde_json::to_vec(&serde_json::json!({
            "width": WIDTH, "height": HEIGHT, "cells": cells,
            "time_ms": now.as_millis(), "frame_ms": FRAME_MS,
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn capture_motion_frames_when_explicitly_requested() {
    let Ok(root) = std::env::var("COCKPIT_MOTION_SNAPSHOT_DIR") else {
        return;
    };
    let root = Path::new(&root);

    // Matching stills use the original readability fixture, including no samples
    // in Live. This makes the measured values identical across both versions.
    for (name, screen) in [("live", Screen::Live), ("statistics", Screen::Statistics)] {
        let (mut app, _) = fixture();
        if screen == Screen::Live {
            app.push(Screen::Configure);
            app.activity = Some(Activity::Test);
        }
        app.push(screen);
        app.live.phase = TestPhase::Download;
        app.live.download_mbps = Some(642.7);
        app.live.ping_ms = Some(10.0);
        app.live.jitter_ms = Some(2.0);
        app.live.speedometer.snap_to_with_peak(642.7, 700.0);
        app.sync_motion(Duration::ZERO, WIDTH, HEIGHT);
        capture(
            root,
            &format!("stills/after-{name}"),
            &mut app,
            Duration::from_millis(1_000),
            Duration::from_secs(6),
        );
    }

    let (mut app, _) = fixture();
    for index in 0..60 {
        if index == 24 || index == 40 {
            press(&mut app, KeyCode::Down);
        }
        let now = Duration::from_millis(index * FRAME_MS);
        capture(
            root,
            &format!("home/{index:03}"),
            &mut app,
            now,
            Duration::ZERO,
        );
    }

    let (mut app, latest) = fixture();
    app.result = None;
    // Drive the same state transitions as an explicit test and engine events.
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Enter);
    app.apply_engine(EngineEvent::IdleLatency {
        ping_ms: 10.0,
        jitter_ms: 2.0,
    });
    let download = [210.0, 480.0, 605.0, 690.0, 580.0, 720.0, 694.0, 642.7];
    let upload = [95.0, 175.0, 230.0, 325.0, 280.0, 340.0, 315.0, 312.3];
    for index in 0..100 {
        let phase = if index < 36 {
            TestPhase::Download
        } else {
            TestPhase::Upload
        };
        if index == 0 || index == 36 {
            app.apply_engine(EngineEvent::PhaseChanged(phase));
        }
        if index < 70 && index % 4 == 0 {
            let values = if index < 36 { &download } else { &upload };
            let offset = if index < 36 { index } else { index - 36 };
            app.apply_engine(EngineEvent::ThroughputSample {
                phase,
                mbps: values[((offset / 4) as usize).min(values.len() - 1)],
            });
            app.apply_engine(EngineEvent::LoadedLatency {
                phase,
                ms: if index < 36 { 20.0 } else { 30.0 },
            });
        }
        if index == 70 {
            app.measured(Ok(latest.clone()));
            app.saved(Ok(()));
        }
        if index < 70 {
            // Twelve equal physics steps per 20 fps frame approximate the actual
            // 240 Hz shell schedule, while remaining entirely deterministic.
            for _ in 0..12 {
                app.live.tick(Duration::from_secs_f64(1.0 / 240.0));
            }
        }
        let now = Duration::from_millis(index * FRAME_MS);
        capture(root, &format!("live/{index:03}"), &mut app, now, now);
    }

    let (mut app, _) = fixture();
    app.push(Screen::Statistics);
    for index in 0..80 {
        if index == 32 {
            press(&mut app, KeyCode::Char('m'));
        }
        if index == 60 {
            press(&mut app, KeyCode::Char('?'));
        }
        let now = Duration::from_millis(index * FRAME_MS);
        capture(
            root,
            &format!("statistics/{index:03}"),
            &mut app,
            now,
            Duration::ZERO,
        );
    }
}
