use std::time::Duration;

use clap::Parser;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};

use super::{
    services::{Archive, Tool},
    state::{Activity, Cockpit, Effect, Load, Modal, Screen},
    theme::Theme,
    view,
};
use crate::{cli::Cli, engine::EngineEvent, model::TestResult, session::TestOptions};

fn app() -> Cockpit {
    Cockpit::new(TestOptions::from(&Cli::parse_from([
        "speedtest",
        "--no-save",
    ])))
}
fn key(app: &mut Cockpit, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn result() -> TestResult {
    serde_json::from_str(include_str!("../../../tests/fixtures/result.json")).unwrap()
}
fn start(app: &mut Cockpit) {
    assert_eq!(key(app, KeyCode::Enter), Effect::None);
    assert_eq!(app.screen(), Screen::Configure);
    assert_eq!(key(app, KeyCode::Enter), Effect::StartTest);
}

#[test]
fn startup_and_navigation_are_offline_until_explicit_start() {
    let mut app = app();
    assert_eq!(app.screen(), Screen::Home);
    assert_eq!(app.activity, None);
    assert!(app.latest().is_none());
    for code in [
        KeyCode::Down,
        KeyCode::Enter,
        KeyCode::Esc,
        KeyCode::Tab,
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Esc,
    ] {
        assert_eq!(key(&mut app, code), Effect::None);
        assert_eq!(app.activity, None);
    }
    app.pages[0].selected = 0;
    start(&mut app);
    assert_eq!(app.activity, Some(Activity::Test));
    assert_eq!(app.screen(), Screen::Live);
}

#[test]
fn back_restores_selection_and_tabs_do_not_grow_the_stack() {
    let mut app = app();
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.screen(), Screen::History);
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.screen(), Screen::Home);
    assert_eq!(app.page().selected, 1);
    for _ in 0..100 {
        key(&mut app, KeyCode::Tab);
        assert!(app.pages.len() <= 2);
    }
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.screen(), Screen::Home);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.pages.len(), 1);
}

#[test]
fn help_is_modal_and_does_not_trigger_actions_beneath_it() {
    let mut app = app();
    key(&mut app, KeyCode::Char('?'));
    key(&mut app, KeyCode::Down);
    assert_eq!(app.page().selected, 0);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.screen(), Screen::Home);
    assert_eq!(app.modal, None);
}

#[test]
fn cancellation_defaults_to_continue_and_stale_events_are_ignored() {
    let mut app = app();
    start(&mut app);
    key(&mut app, KeyCode::Esc);
    assert_eq!(
        app.modal,
        Some(Modal::Cancel {
            quit: false,
            confirm: false
        })
    );
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.activity, Some(Activity::Test));
    key(&mut app, KeyCode::Esc);
    assert_eq!(key(&mut app, KeyCode::Char('y')), Effect::Cancel);
    assert_eq!(app.screen(), Screen::Configure);
    assert_eq!(app.activity, None);
    app.apply_engine(EngineEvent::Complete(result()));
    app.measured(Ok(result()));
    app.saved(Ok(()));
    assert!(app.result.is_none());
    assert!(app.recent.is_none());
}

#[test]
fn q_confirms_running_work_but_quits_idle_menu_successfully() {
    let mut app = app();
    assert_eq!(key(&mut app, KeyCode::Char('q')), Effect::Quit);
    start(&mut app);
    assert_eq!(key(&mut app, KeyCode::Char('q')), Effect::None);
    assert_eq!(key(&mut app, KeyCode::Char('y')), Effect::Quit);
    assert_eq!(app.activity, None);
}

#[test]
fn result_return_not_complete_event_controls_saving() {
    let mut app = app();
    start(&mut app);
    app.apply_engine(EngineEvent::Complete(result()));
    assert_eq!(app.activity, Some(Activity::Test));
    assert_eq!(app.screen(), Screen::Live);
    key(&mut app, KeyCode::Esc);
    app.measured(Ok(result()));
    assert_eq!(app.modal, None);
    assert_eq!(app.activity, Some(Activity::Saving));
    assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
    assert_eq!(app.activity, Some(Activity::Saving));
    app.saved(Ok(()));
    assert_eq!(app.screen(), Screen::Results);
    assert!(app.latest().is_some());
    assert!(app.save_notice.starts_with("NOT SAVED"));
    assert_eq!(app.activity, None);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.screen(), Screen::Configure);
}

#[test]
fn save_failure_retains_result_and_never_claims_saved() {
    let mut app = app();
    start(&mut app);
    app.measured(Ok(result()));
    app.saved(Err("fixture permission denied".into()));
    assert_eq!(app.screen(), Screen::Results);
    assert!(app.result.is_some());
    assert!(app.save_notice.starts_with("SAVE FAILED"));
    assert!(app.save_notice.contains("permission denied"));
}

#[test]
fn retry_replaces_failure_instead_of_accumulating_pages() {
    let mut app = app();
    start(&mut app);
    for _ in 0..20 {
        app.measured(Err("fixture timeout".into()));
        assert_eq!(app.screen(), Screen::Failure);
        assert_eq!(key(&mut app, KeyCode::Char('r')), Effect::StartTest);
        assert_eq!(app.pages.len(), 3);
        assert!(app.live.error.is_none());
        assert!(app.live.result.is_none());
    }
}

#[test]
fn completed_result_rerun_starts_a_fresh_workflow() {
    let mut app = app();
    start(&mut app);
    for _ in 0..10 {
        app.measured(Ok(result()));
        app.saved(Ok(()));
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.screen(), Screen::Configure);
        assert_eq!(key(&mut app, KeyCode::Enter), Effect::StartTest);
        assert_eq!(app.pages.len(), 3);
    }
}

#[test]
fn timing_controls_respect_cli_ranges_and_preserve_export_configuration() {
    let mut app = app();
    app.options.output = Some("explicit.csv".into());
    app.options.format = crate::cli::OutputFormat::Csv;
    app.push(Screen::Settings);
    for (index, maximum, minimum) in [(1, 30, 3), (2, 16, 1), (3, 240, 30), (5, 600, 1)] {
        app.page_mut().selected = index;
        for _ in 0..700 {
            key(&mut app, KeyCode::Char('+'));
        }
        let value = match index {
            1 => app.options.duration,
            2 => u64::from(app.options.streams),
            3 => u64::from(app.options.fps),
            _ => app.options.timeout,
        };
        assert_eq!(value, maximum);
        for _ in 0..700 {
            key(&mut app, KeyCode::Char('-'));
        }
        let value = match index {
            1 => app.options.duration,
            2 => u64::from(app.options.streams),
            3 => u64::from(app.options.fps),
            _ => app.options.timeout,
        };
        assert_eq!(value, minimum);
    }
    app.page_mut().selected = 7;
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.options.output.as_deref(),
        Some(std::path::Path::new("explicit.csv"))
    );
    assert!(matches!(app.options.format, crate::cli::OutputFormat::Csv));
    assert_eq!(
        (app.options.duration, app.options.streams, app.options.fps),
        (8, 2, 60)
    );
}

#[test]
fn tools_require_separate_start_and_retry_without_changing_navigation() {
    let mut app = app();
    app.push(Screen::Dns);
    app.page_mut().selected = 2;
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.tool, Some(Tool::DnsTest));
    assert_eq!(app.activity, None);
    assert_eq!(
        key(&mut app, KeyCode::Enter),
        Effect::StartTool(Tool::DnsTest)
    );
    app.tool_finished(Err("fixture failure".into()));
    assert!(matches!(app.report, Some(Load::Failed(_))));
    assert_eq!(
        key(&mut app, KeyCode::Char('r')),
        Effect::StartTool(Tool::DnsTest)
    );
    assert_eq!(app.pages.len(), 3);
}

#[test]
fn history_reload_clamps_selection_and_corruption_does_not_disable_testing() {
    let mut app = app();
    app.push(Screen::History);
    app.page_mut().selected = 99;
    app.set_history(Ok(Archive::from_results(vec![result()])));
    assert_eq!(app.page().selected, 0);
    app.set_history(Err("invalid history record on line 4".into()));
    assert_eq!(key(&mut app, KeyCode::Char('r')), Effect::LoadHistory);
    assert!(matches!(app.history, Load::Loading));
    key(&mut app, KeyCode::Esc);
    start(&mut app);
}

#[test]
fn key_release_and_activation_repeat_cannot_start_a_test() {
    let mut app = app();
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let mut event = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        event.kind = kind;
        assert_eq!(app.key(event), Effect::None);
        assert_eq!(app.screen(), Screen::Home);
    }
}

fn render(app: &mut Cockpit, width: u16, height: u16) -> (String, ratatui::buffer::Buffer) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| view::draw(frame, app, Theme::rgb(), Duration::from_secs(2)))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let mut text = String::new();
    for y in 0..height {
        for x in 0..width {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    (text, buffer)
}

#[test]
fn every_screen_renders_at_80x24_and_survives_extreme_resize() {
    let mut app = app();
    app.set_history(Ok(Archive::from_results(vec![result(), result()])));
    app.result = Some(result());
    app.tool = Some(Tool::DnsList);
    app.report = Some(Load::Ready("DNS PROVIDERS\nSample local report".into()));
    for screen in [
        Screen::Home,
        Screen::Configure,
        Screen::Live,
        Screen::Results,
        Screen::History,
        Screen::Statistics,
        Screen::Compare,
        Screen::Dns,
        Screen::Diagnostics,
        Screen::Settings,
        Screen::Tool,
        Screen::Failure,
    ] {
        app.push(screen);
        for (width, height) in [(80, 24), (100, 30), (160, 48), (79, 23), (40, 12), (1, 1)] {
            let (text, _) = render(&mut app, width, height);
            assert!(!text.is_empty());
            if width >= 80 {
                assert!(text.contains("SPEEDTEST"));
                assert!(text.contains("quit"));
            }
        }
        app.pages.pop();
    }
    for modal in [
        Modal::Help,
        Modal::Cancel {
            quit: true,
            confirm: false,
        },
    ] {
        app.modal = Some(modal);
        for (w, h) in [(80, 24), (40, 12), (1, 1)] {
            render(&mut app, w, h);
        }
    }
}

#[test]
fn dashboard_and_history_empty_loading_error_states_are_distinct() {
    let mut app = app();
    assert!(render(&mut app, 80, 24).0.contains("READING LOCAL HISTORY"));
    app.set_history(Ok(Archive::from_results(vec![])));
    let text = render(&mut app, 80, 24).0;
    assert!(text.contains("No tests yet"));
    assert!(text.contains("Run Speed Test"));
    assert!(text.contains("NETWORK NOT PROBED"));
    app.set_history(Err("corrupt record".into()));
    assert!(render(&mut app, 80, 24).0.contains("HISTORY UNAVAILABLE"));
    app.push(Screen::History);
    assert!(render(&mut app, 80, 24).0.contains("corrupt record"));
}

#[test]
fn report_scroll_clamps_after_resize_and_external_controls_are_removed() {
    let mut app = app();
    app.push(Screen::Tool);
    app.tool = Some(Tool::DnsList);
    app.report = Some(Load::Ready("sentinel\u{202e}\x1b\r\u{009b}\n".repeat(30)));
    app.page_mut().scroll = u16::MAX;
    let text = render(&mut app, 80, 24).0;
    assert!(!text.contains(['\u{202e}', '\x1b', '\r', '\u{009b}']));
    assert!(app.page().scroll < 40);
    render(&mut app, 100, 50);
    assert_eq!(app.page().scroll, 0);
}

#[test]
fn selected_history_row_stays_visible_and_opens_without_saving() {
    let mut app = app();
    let results = (0..40)
        .map(|i| {
            let mut result = result();
            result.backend = format!("row-{i:02}");
            result
        })
        .collect();
    app.set_history(Ok(Archive::from_results(results)));
    app.push(Screen::History);
    app.page_mut().selected = 39;
    let text = render(&mut app, 80, 24).0;
    assert!(text.contains("row-00"));
    assert!(text.contains("QUALITY"));
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.result.as_ref().unwrap().backend, "row-00");
    assert_eq!(app.activity, None);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.page().selected, 39);
}

#[test]
fn capture_review_frames_when_explicitly_requested() {
    let Ok(root) = std::env::var("COCKPIT_SNAPSHOT_DIR") else {
        return;
    };
    std::fs::create_dir_all(&root).unwrap();
    let mut app = app();
    app.set_history(Ok(Archive::from_results(vec![])));
    for (name, screen) in [
        ("home-empty", Screen::Home),
        ("configuration", Screen::Configure),
        ("dns", Screen::Dns),
    ] {
        if screen != Screen::Home {
            app.push(screen);
        }
        let (text, buffer) = render(&mut app, 80, 24);
        save_frame(&root, name, &text, &buffer);
        if screen != Screen::Home {
            app.pages.pop();
        }
    }
    app.set_history(Ok(Archive::from_results(vec![result(), result()])));
    app.result = Some(result());
    for (name, screen) in [
        ("home-recent", Screen::Home),
        ("history", Screen::History),
        ("results", Screen::Results),
        ("statistics", Screen::Statistics),
        ("compare", Screen::Compare),
    ] {
        if screen == Screen::Compare {
            app.push(Screen::History);
        }
        if screen != Screen::Home {
            app.push(screen);
        }
        let (text, buffer) = render(&mut app, 80, 24);
        save_frame(&root, name, &text, &buffer);
        if screen != Screen::Home {
            app.pages.pop();
        }
        if screen == Screen::Compare {
            app.pages.pop();
        }
    }
    app.push(Screen::Configure);
    app.push(Screen::Live);
    app.activity = Some(Activity::Test);
    app.live.apply(EngineEvent::ThroughputSample {
        phase: crate::model::TestPhase::Download,
        mbps: 642.7,
    });
    app.live.speedometer.snap_to_with_peak(642.7, 700.0);
    let (text, buffer) = render(&mut app, 100, 30);
    save_frame(&root, "live", &text, &buffer);
}

fn save_frame(root: &str, name: &str, text: &str, buffer: &ratatui::buffer::Buffer) {
    let root = std::path::Path::new(root);
    std::fs::write(root.join(format!("{name}.txt")), text).unwrap();
    let cells: Vec<_> = buffer
        .content
        .iter()
        .map(|cell| {
            serde_json::json!({
                "symbol":cell.symbol(), "fg":format!("{:?}",cell.fg), "bg":format!("{:?}",cell.bg),
                "bold":cell.modifier.contains(ratatui::style::Modifier::BOLD),
            })
        })
        .collect();
    std::fs::write(
        root.join(format!("{name}.json")),
        serde_json::to_string(&serde_json::json!({
            "width":buffer.area.width,"height":buffer.area.height,"cells":cells,
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn small_terminal_keeps_cancel_available_but_blocks_hidden_start_controls() {
    let mut app = app();
    for code in [KeyCode::Enter, KeyCode::Tab, KeyCode::Down] {
        assert_eq!(
            app.key_at_size(KeyEvent::new(code, KeyModifiers::NONE), 60, 18),
            Effect::None
        );
    }
    assert_eq!(app.screen(), Screen::Home);
    assert_eq!(app.activity, None);
    start(&mut app);
    app.key_at_size(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), 60, 18);
    assert_eq!(
        app.key_at_size(
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
            60,
            18
        ),
        Effect::Cancel
    );
}

#[test]
fn retry_key_never_changes_settings_or_starts_a_hidden_action() {
    let mut app = app();
    app.push(Screen::Configure);
    assert_eq!(key(&mut app, KeyCode::Char('r')), Effect::None);
    assert_eq!(app.activity, None);
    app.page_mut().selected = 2;
    let duration = app.options.duration;
    key(&mut app, KeyCode::Char('r'));
    assert_eq!(app.options.duration, duration);
}

#[test]
fn empty_state_preserves_paragraphs_and_busy_footer_lists_only_available_actions() {
    let mut app = app();
    app.set_history(Ok(Archive::from_results(vec![])));
    let (text, _) = render(&mut app, 80, 24);
    assert!(text.contains("Your connection has not been probed."));
    assert!(!text.contains("here.Your"));
    start(&mut app);
    let (text, _) = render(&mut app, 80, 24);
    assert!(text.contains("Navigation resumes when the task finishes."));
    assert!(!text.contains("Tab /"));
}

#[test]
fn basic_color_fallback_preserves_text_labels_and_selection_markers() {
    let mut app = app();
    app.set_history(Ok(Archive::from_results(vec![])));
    for theme in [Theme::ansi(), Theme::indexed()] {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| view::draw(frame, &mut app, theme, Duration::ZERO))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Run Speed Test"));
        assert!(text.contains("NETWORK NOT PROBED"));
        assert!(text.contains("> Home"));
    }
}
