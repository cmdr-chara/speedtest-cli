//! Quick start is an explicit network action; ordinary navigation remains offline.
use clap::Parser;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{
    services::Archive,
    state::{Activity, Cockpit, Effect, Modal, Screen},
};
use crate::{cli::Cli, model::TestResult, session::TestOptions};

fn app() -> Cockpit {
    Cockpit::new(TestOptions::from(&Cli::parse_from([
        "speedtest",
        "--backend",
        "librespeed",
        "--family",
        "ipv4",
        "--server-id",
        "7",
        "--exclude-server-id",
        "12",
        "--source-ip",
        "127.0.0.1",
        "--duration",
        "3",
        "--streams",
        "4",
        "--fps",
        "144",
        "--timeout",
        "60",
        "--no-save",
        "--output",
        "explicit-export.csv",
        "--format",
        "csv",
    ])))
}

fn key(app: &mut Cockpit, code: KeyCode) -> Effect {
    app.key_at_size(KeyEvent::new(code, KeyModifiers::NONE), 120, 38)
}

fn result() -> TestResult {
    serde_json::from_str(include_str!("../../../tests/fixtures/result.json")).unwrap()
}

#[test]
fn quick_start_keeps_enter_and_ordinary_navigation_offline() {
    let mut app = app();
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.screen(), Screen::Configure);
    assert_eq!(app.activity, None);
    assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);

    for code in [KeyCode::Down, KeyCode::Enter, KeyCode::Esc, KeyCode::Tab] {
        assert_eq!(key(&mut app, code), Effect::None);
        assert_eq!(app.activity, None);
    }

    app.result = Some(result());
    app.push(Screen::Results);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.screen(), Screen::Configure);
    assert_eq!(app.pages.len(), 2);
    assert_eq!(app.activity, None);
}

#[test]
fn home_space_starts_once_with_every_current_option_preserved() {
    let mut app = app();
    let options_before = format!("{:?}", app.options);
    app.page_mut().selected = 4;
    app.failure = "old failure".into();
    app.result = Some(result());
    assert_eq!(key(&mut app, KeyCode::Char(' ')), Effect::StartTest);
    assert_eq!(app.screen(), Screen::Live);
    assert_eq!(app.activity, Some(Activity::Test));
    assert_eq!(format!("{:?}", app.options), options_before);
    assert!(app.result.is_none());
    assert!(app.failure.is_empty());

    for _ in 0..3 {
        assert_eq!(key(&mut app, KeyCode::Char(' ')), Effect::None);
    }
    assert_eq!(app.pages.len(), 2);
}

#[test]
fn repeated_tests_replace_results_and_keep_the_original_back_destination() {
    for origin in [Screen::Home, Screen::Configure, Screen::History] {
        let mut app = app();
        if origin != Screen::Home {
            app.push(origin);
        }
        app.result = Some(result());
        app.push(Screen::Results);
        let depth = app.pages.len();
        let options_before = format!("{:?}", app.options);

        for _ in 0..32 {
            assert_eq!(key(&mut app, KeyCode::Char(' ')), Effect::StartTest);
            assert_eq!(app.screen(), Screen::Live);
            assert_eq!(app.pages.len(), depth);
            assert_eq!(format!("{:?}", app.options), options_before);
            app.measured(Ok(result()));
            assert_eq!(key(&mut app, KeyCode::Char(' ')), Effect::None);
            app.saved(Ok(()));
            assert_eq!(app.screen(), Screen::Results);
            assert_eq!(app.pages.len(), depth);
        }
        assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
        assert_eq!(app.screen(), origin);
    }
}

#[test]
fn historical_result_quick_start_uses_current_settings_not_saved_measurements() {
    let mut app = app();
    let saved = result();
    assert_ne!(saved.backend, app.options.backend_label());
    app.set_history(Ok(Archive::from_results(vec![saved])));
    app.push(Screen::History);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.screen(), Screen::Results);
    let options_before = format!("{:?}", app.options);
    assert_eq!(key(&mut app, KeyCode::Char(' ')), Effect::StartTest);
    assert_eq!(format!("{:?}", app.options), options_before);
    assert_eq!(app.options.backend_label(), "LibreSpeed");
}

#[test]
fn quick_start_is_blocked_while_busy_or_behind_a_modal() {
    for screen in [Screen::Home, Screen::Results] {
        for activity in [Activity::Test, Activity::Saving, Activity::Tool] {
            let mut app = app();
            if screen != Screen::Home {
                app.push(screen);
            }
            app.activity = Some(activity);
            assert_eq!(key(&mut app, KeyCode::Char(' ')), Effect::None);
            assert_eq!(app.screen(), screen);
            assert_eq!(app.activity, Some(activity));
        }
        for modal in [
            Modal::Help,
            Modal::TextSize,
            Modal::Cancel {
                quit: false,
                confirm: true,
            },
        ] {
            let mut app = app();
            if screen != Screen::Home {
                app.push(screen);
            }
            app.modal = Some(modal);
            assert_eq!(key(&mut app, KeyCode::Char(' ')), Effect::None);
            assert_eq!(app.screen(), screen);
            assert_eq!(app.activity, None);
            assert_eq!(app.modal, Some(modal));
        }
    }
}

#[test]
fn quick_start_requires_a_fresh_visible_unmodified_space_press() {
    for screen in [Screen::Home, Screen::Results] {
        for (width, height) in [(79, 24), (80, 23), (1, 1)] {
            let mut app = app();
            if screen != Screen::Home {
                app.push(screen);
            }
            assert_eq!(
                app.key_at_size(
                    KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                    width,
                    height,
                ),
                Effect::None
            );
            assert_eq!(app.activity, None);
            assert_eq!(app.screen(), screen);
        }
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let mut app = app();
            if screen != Screen::Home {
                app.push(screen);
            }
            assert_eq!(
                app.key(KeyEvent::new_with_kind(
                    KeyCode::Char(' '),
                    KeyModifiers::NONE,
                    kind,
                )),
                Effect::None
            );
            assert_eq!(app.activity, None);
        }
        for modifiers in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
            let mut app = app();
            if screen != Screen::Home {
                app.push(screen);
            }
            assert_eq!(
                app.key(KeyEvent::new(KeyCode::Char(' '), modifiers)),
                Effect::None
            );
            assert_eq!(app.activity, None);
        }
    }
}

#[test]
fn spaces_in_history_search_remain_text_and_other_sections_do_not_quick_start() {
    let mut search = app();
    search.push(Screen::History);
    assert_eq!(key(&mut search, KeyCode::Char('/')), Effect::None);
    assert_eq!(key(&mut search, KeyCode::Char(' ')), Effect::None);
    assert_eq!(search.history_view.query, " ");
    assert_eq!(search.activity, None);

    for screen in [
        Screen::History,
        Screen::Statistics,
        Screen::Compare,
        Screen::Dns,
        Screen::Diagnostics,
        Screen::Tool,
        Screen::Failure,
    ] {
        let mut app = app();
        app.push(screen);
        assert_eq!(key(&mut app, KeyCode::Char(' ')), Effect::None);
        assert_eq!(app.activity, None);
        assert_eq!(app.screen(), screen);
    }
}
