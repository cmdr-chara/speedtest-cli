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

fn comparison_results() -> Vec<TestResult> {
    [
        ("fixture-before", 100.0, 20.0, 30.0, 8.0, 50.0, 60),
        ("fixture-after", 400.0, 50.0, 12.0, 2.0, 8.0, 90),
    ]
    .into_iter()
    .enumerate()
    .map(
        |(index, (backend, down, up, idle, jitter, loaded, score))| {
            let mut result = result();
            result.timestamp += chrono::Duration::hours(index as i64);
            result.backend = backend.into();
            result.download.mbps = down;
            result.upload.mbps = up;
            result.latency.idle_ms = idle;
            result.latency.jitter_ms = jitter;
            result.analysis = Some(crate::analysis::build_network_analysis(
                &[idle; 12],
                &[idle + loaded; 12],
                &[idle + loaded; 12],
                &result.latency,
                &result.download,
                &result.upload,
            ));
            let quality = &mut result.analysis.as_mut().unwrap().quality;
            quality.score = score;
            quality.grade = if index == 0 {
                crate::model::QualityGrade::D
            } else {
                crate::model::QualityGrade::A
            };
            quality.bufferbloat.worst_increase_ms = Some(loaded);
            result
        },
    )
    .collect()
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
        let mut x = 0;
        while x < width {
            let symbol = buffer[(x, y)].symbol();
            text.push_str(symbol);
            // Ratatui reserves the following cell for a wide glyph. Do not
            // mistake that placeholder for a visible space in translated text.
            x += ratatui::text::Line::from(symbol).width().max(1) as u16;
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
    assert!(!text.contains("LAST RESULT AVAILABLE"));
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
    let small_scroll = app.page().scroll;
    let text = render(&mut app, 100, 50).0;
    // Growing the terminal makes the whole report available instead of leaving
    // it scrolled inside a fixed-height workspace.
    assert!(app.page().scroll < small_scroll);
    assert_eq!(app.page().scroll, 0);
    assert_eq!(text.matches("sentinel").count(), 30);
    render(&mut app, 100, 100);
    assert_eq!(app.page().scroll, 0);
    app.page_mut().scroll = u16::MAX;
    render(&mut app, 80, 24);
    assert!(app.page().scroll > 0);
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
fn history_pages_and_endpoints_follow_the_visible_viewport_after_resize() {
    let mut app = app();
    let records = (0..40)
        .map(|i| {
            let mut result = result();
            result.backend = format!("row-{i:02}");
            result
        })
        .collect();
    app.set_history(Ok(Archive::from_results(records)));
    app.push(Screen::History);
    render(&mut app, 80, 24);
    let small_page = app.history_page_size;
    assert!(small_page > 1);
    assert_eq!(key(&mut app, KeyCode::PageDown), Effect::None);
    let first_page = app.page().selected;
    assert_eq!(first_page, small_page);
    key(&mut app, KeyCode::PageDown);
    assert!(app.page().selected > first_page);
    key(&mut app, KeyCode::PageUp);
    assert_eq!(app.page().selected, first_page);
    key(&mut app, KeyCode::End);
    assert_eq!(app.page().selected, 39);
    let text = render(&mut app, 80, 24).0;
    assert!(text.contains("row-00"));
    assert!(text.contains("Run 40 of 40"));
    key(&mut app, KeyCode::PageDown);
    assert_eq!(app.page().selected, 39);
    key(&mut app, KeyCode::Home);
    assert_eq!(app.page().selected, 0);
    assert!(render(&mut app, 120, 38).0.contains("row-39"));
    assert!(app.history_page_size > small_page);
    key(&mut app, KeyCode::PageDown);
    let selected = app.page().selected;
    let expected = format!("row-{:02}", 39 - selected);
    assert!(render(&mut app, 120, 38).0.contains(&expected));
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.result.as_ref().unwrap().backend, expected);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.page().selected, selected);
    assert_eq!(app.activity, None);
}

#[test]
fn reload_restores_complete_record_identity_and_duplicate_occurrence() {
    let mut app = app();
    // Same timestamp, backend and server; metrics distinguish these records.
    let records: Vec<_> = [10.0, 20.0, 20.0, 30.0]
        .into_iter()
        .map(|value| {
            let mut result = result();
            result.download.mbps = value;
            result
        })
        .collect();
    app.set_history(Ok(Archive::from_results(records.clone())));
    app.push(Screen::History);
    app.page_mut().selected = 2; // The older occurrence of the duplicate 20 Mbps run.
    assert_eq!(key(&mut app, KeyCode::Char('r')), Effect::LoadHistory);
    let mut appended = records;
    let mut new = result();
    new.download.mbps = 40.0;
    appended.push(new);
    app.set_history(Ok(Archive::from_results(appended.clone())));
    assert_eq!(app.page().selected, 3);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.result.as_ref().unwrap().download.mbps, 20.0);
    key(&mut app, KeyCode::Esc);
    assert_eq!(key(&mut app, KeyCode::Char('r')), Effect::LoadHistory);
    app.set_history(Err("fixture temporary read failure".into()));
    assert_eq!(key(&mut app, KeyCode::Char('r')), Effect::LoadHistory);
    app.set_history(Ok(Archive::from_results(appended)));
    assert_eq!(app.page().selected, 3);
    assert_eq!(key(&mut app, KeyCode::Char('r')), Effect::LoadHistory);
    app.set_history(Ok(Archive::from_results(vec![result()])));
    assert_eq!(app.page().selected, 0);
    assert!(render(&mut app, 80, 24).0.contains("Run 1 of 1"));
}

#[test]
fn baseline_comparison_uses_explicit_snapshots_and_preserves_history_navigation() {
    let mut app = app();
    let records = comparison_results();
    let mut newest = records[1].clone();
    newest.backend = "fixture-newest".into();
    newest.download.mbps = 700.0;
    app.set_history(Ok(Archive::from_results(vec![
        records[0].clone(),
        records[1].clone(),
        newest.clone(),
    ])));
    app.push(Screen::History);
    app.page_mut().selected = 1;
    assert_eq!(key(&mut app, KeyCode::Char('c')), Effect::None);
    let compared = app.comparison.as_ref().unwrap();
    assert_eq!(compared.before.backend, "fixture-before");
    assert_eq!(compared.after.backend, "fixture-after");
    assert_eq!(compared.metrics.download_mbps.absolute_change, 300.0);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.page().selected, 1);
    key(&mut app, KeyCode::End);
    assert_eq!(key(&mut app, KeyCode::Char('c')), Effect::None);
    assert_eq!(app.screen(), Screen::History);
    assert!(app.notice.starts_with("No older run"));
    assert_eq!(key(&mut app, KeyCode::Char('b')), Effect::None);
    assert!(app.baseline.is_some());
    key(&mut app, KeyCode::Char('c'));
    assert_eq!(app.screen(), Screen::History);
    assert!(app.notice.starts_with("Select another run"));
    key(&mut app, KeyCode::Home);
    assert_eq!(key(&mut app, KeyCode::Char('c')), Effect::None);
    assert_eq!(
        app.comparison.as_ref().unwrap().before.backend,
        "fixture-before"
    );
    assert_eq!(
        app.comparison.as_ref().unwrap().after.backend,
        "fixture-newest"
    );
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.page().selected, 0);
    // Pin survives removal from history, and Compare remains the chosen snapshot.
    assert_eq!(key(&mut app, KeyCode::Char('r')), Effect::LoadHistory);
    app.set_history(Ok(Archive::from_results(vec![newest])));
    key(&mut app, KeyCode::Char('c'));
    assert_eq!(
        app.comparison.as_ref().unwrap().before.backend,
        "fixture-before"
    );
    app.set_history(Err("fixture reload failure".into()));
    let text = render(&mut app, 80, 24).0;
    assert!(text.contains("fixture-before") && text.contains("fixture-newest"));
    assert!(!text.contains("HISTORY UNAVAILABLE"));
    assert_eq!(app.activity, None);
}

#[test]
fn baseline_toggle_is_offline_and_hidden_or_repeated_keys_cannot_change_it() {
    let mut app = app();
    app.set_history(Ok(Archive::from_results(comparison_results())));
    app.push(Screen::History);
    let options = format!("{:?}", app.options);
    for code in [
        KeyCode::Char('b'),
        KeyCode::Char('c'),
        KeyCode::Home,
        KeyCode::End,
    ] {
        assert_eq!(
            app.key_at_size(KeyEvent::new(code, KeyModifiers::NONE), 79, 23),
            Effect::None
        );
    }
    assert!(app.baseline.is_none());
    let mut repeated = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE);
    repeated.kind = KeyEventKind::Repeat;
    app.key(repeated);
    assert!(app.baseline.is_none());
    key(&mut app, KeyCode::Char('?'));
    key(&mut app, KeyCode::Char('b'));
    assert!(app.baseline.is_none());
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('b'));
    assert!(render(&mut app, 80, 24).0.contains("BASELINE"));
    key(&mut app, KeyCode::Char('b'));
    assert!(app.baseline.is_none());
    assert_eq!(format!("{:?}", app.options), options);
    assert_eq!(app.activity, None);
}

#[test]
fn comparison_shows_every_metric_source_and_localized_aligned_values() {
    use crate::i18n::{text, Language};
    for language in Language::ALL {
        let mut app = app();
        app.language = language;
        app.set_history(Ok(Archive::from_results(comparison_results())));
        app.push(Screen::History);
        key(&mut app, KeyCode::Char('c'));
        for (width, height) in [(80, 24), (120, 38)] {
            key(&mut app, KeyCode::Home);
            let rendered = render(&mut app, width, height).0;
            assert!(
                rendered.contains("fixture-before"),
                "{}: {rendered}",
                language.code()
            );
            assert!(rendered.contains("fixture-after"));
            for label in [
                "Download",
                "Upload",
                "Idle latency",
                "Jitter",
                "Quality",
                "Loaded increase",
            ] {
                assert!(
                    rendered.contains(&text(language, label)),
                    "{} missing {label}: {rendered}",
                    language.code()
                );
            }
            for value in [
                "100.0 Mbps",
                "400.0 Mbps",
                "60/100",
                "90/100",
                "50.0 ms",
                "8.0 ms",
            ] {
                assert!(
                    rendered.contains(value),
                    "{} missing {value}: {rendered}",
                    language.code()
                );
            }
            let before_header = text(language, "BEFORE");
            let header = rendered
                .lines()
                .find(|line| line.contains(&text(language, "METRIC")))
                .unwrap();
            let row = rendered
                .lines()
                .find(|line| line.contains("100.0 Mbps"))
                .unwrap();
            let end = |line: &str, value: &str| {
                let prefix = &line[..line.find(value).unwrap() + value.len()];
                ratatui::text::Line::from(prefix).width()
            };
            assert_eq!(end(header, &before_header), end(row, "100.0 Mbps"));
            assert_eq!(
                end(header, &text(language, "AFTER")),
                end(row, "400.0 Mbps")
            );
        }
        key(&mut app, KeyCode::End);
        let rendered = render(&mut app, 80, 24).0;
        assert!(rendered.contains("localhost"));
        assert!(app.page().scroll > 0);
        key(&mut app, KeyCode::Home);
        assert_eq!(app.page().scroll, 0);
    }
}

#[test]
fn comparison_keeps_missing_and_oversized_values_readable() {
    let mut app = app();
    let mut records = comparison_results();
    records[0].analysis = None;
    records[0].download.mbps = 1_000_000_000_000_000_000.0;
    records[1].download.mbps = 2_000_000_000_000_000_000.0;
    app.set_history(Ok(Archive::from_results(records)));
    app.push(Screen::History);
    key(&mut app, KeyCode::Char('c'));
    let mut content = String::new();
    for _ in 0..8 {
        content.push_str(&render(&mut app, 80, 24).0);
        key(&mut app, KeyCode::PageDown);
    }
    assert!(content.contains("1000000000000000000.0 Mbps"));
    assert!(content.contains("2000000000000000000.0 Mbps"));
    assert!(content.contains("n/a"));
    assert!(content.contains("90/100"));
    assert!(content.contains("Loaded increase"));
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
    app.set_history(Ok(Archive::from_results(comparison_results())));
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
    app.push(Screen::History);
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Char('b'));
    key(&mut app, KeyCode::Home);
    for (width, height) in [(80, 24), (120, 38)] {
        let (text, buffer) = render(&mut app, width, height);
        save_frame(
            &root,
            &format!("history-pinned-{width}x{height}"),
            &text,
            &buffer,
        );
    }
    key(&mut app, KeyCode::Char('c'));
    for (width, height) in [(80, 24), (120, 38)] {
        let (text, buffer) = render(&mut app, width, height);
        save_frame(
            &root,
            &format!("compare-pinned-{width}x{height}"),
            &text,
            &buffer,
        );
    }
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Esc);
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
                "reversed":cell.modifier.contains(ratatui::style::Modifier::REVERSED),
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
        assert!(!text.contains("LAST RESULT AVAILABLE"));
        assert!(text.contains("> Home"));
    }
}

#[test]
fn terminal_colors_are_defaults_not_guessed_from_color_capability() {
    use super::theme::{ColorDepth, Palette};
    use ratatui::style::Color;
    assert_eq!(app().palette, Palette::Terminal);
    for depth in [
        ColorDepth::TrueColor,
        ColorDepth::Indexed,
        ColorDepth::Basic,
    ] {
        let theme = Theme::resolve(Palette::Terminal, depth);
        assert_eq!(theme.background, Color::Reset);
        assert_eq!(theme.text, Color::Reset);
        assert_eq!(theme.muted, Color::Reset);
        assert_eq!(theme.focus, Color::Cyan);
        assert_eq!(
            Theme::resolve(Palette::Graphite, ColorDepth::Basic).background,
            Color::Reset
        );
    }
}

#[test]
fn every_native_and_monochrome_screen_avoids_hard_coded_color_pairs() {
    use ratatui::style::{Color, Modifier};
    let mut app = app();
    app.set_history(Ok(Archive::from_results(vec![result(), result()])));
    app.result = Some(result());
    app.tool = Some(Tool::DnsList);
    app.report = Some(Load::Ready("Example report".into()));
    app.live.speedometer.snap_to_with_peak(34.7, 100.0);
    for theme in [Theme::ansi(), Theme::monochrome()] {
        for screen in [
            Screen::Home,
            Screen::Configure,
            Screen::Settings,
            Screen::Live,
            Screen::Results,
            Screen::History,
            Screen::Statistics,
            Screen::Compare,
            Screen::Dns,
            Screen::Diagnostics,
            Screen::Tool,
            Screen::Failure,
        ] {
            app.push(screen);
            for (width, height) in [(80, 24), (120, 38), (180, 48)] {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| view::draw(frame, &mut app, theme, Duration::ZERO))
                    .unwrap();
                for cell in terminal.backend().buffer().content() {
                    assert_eq!(cell.bg, Color::Reset, "{screen:?} must inherit background");
                    assert!(!matches!(
                        cell.fg,
                        Color::Rgb(..)
                            | Color::Indexed(_)
                            | Color::Black
                            | Color::White
                            | Color::Gray
                            | Color::DarkGray
                    ));
                    assert!(!cell.modifier.contains(Modifier::DIM));
                    if theme.focus == Color::Reset {
                        assert_eq!(cell.fg, Color::Reset);
                    }
                }
            }
            if app.pages.len() > 1 {
                app.pages.pop();
            }
        }
    }
}

#[test]
fn home_highlight_is_one_label_not_a_three_row_painted_rectangle() {
    use ratatui::style::Modifier;
    let mut app = app();
    app.set_history(Ok(Archive::from_results(vec![])));
    for (width, height) in [(80, 24), (120, 38), (180, 48)] {
        let (text, buffer) = render(&mut app, width, height);
        let y = text
            .lines()
            .position(|line| line.contains("Run Speed Test"))
            .unwrap() as u16;
        let highlighted = (0..width)
            .filter(|x| buffer[(*x, y)].modifier.contains(Modifier::REVERSED))
            .count();
        assert_eq!(highlighted, " Run Speed Test ".len());
        for row in [y + 1, y + 2] {
            assert!((0..width).all(|x| !buffer[(x, row)].modifier.contains(Modifier::REVERSED)));
        }
    }
}

#[test]
fn dashboard_uses_full_width_and_keeps_controls_near_content_after_resize() {
    let mut app = app();
    app.set_history(Ok(Archive::from_results(vec![])));
    for (width, height) in [(80, 24), (120, 38), (214, 52), (300, 100), (80, 24)] {
        let (text, buffer) = render(&mut app, width, height);
        assert_eq!(buffer[(2, 1)].symbol(), "S", "header stays at top left");
        assert_eq!(
            buffer[(width - 3, 4)].symbol(),
            "─",
            "tabs span the terminal"
        );
        let footer = text.lines().position(|line| line.contains("quit")).unwrap();
        assert!(footer <= usize::from(height - 2));
        if height >= 38 {
            assert!(
                footer < usize::from(height - 5),
                "Home does not push controls across blank rows"
            );
        }
        assert_eq!(app.screen(), Screen::Home);
        assert_eq!(app.activity, None);
    }
}

#[test]
fn home_keeps_speed_columns_together_and_result_action_visible_after_long_findings() {
    let mut app = app();
    let mut saved = comparison_results().pop().unwrap();
    let finding = saved
        .analysis
        .as_mut()
        .unwrap()
        .quality
        .findings
        .first_mut()
        .unwrap();
    finding.title = "A detailed finding title ".repeat(30);
    finding.evidence = "Detailed evidence ".repeat(80);
    app.set_history(Ok(Archive::from_results(vec![saved])));
    for (width, height) in [(80, 24), (120, 38), (214, 52)] {
        let text = render(&mut app, width, height).0;
        assert!(text.contains("v  Open result"));
        let metrics = text
            .lines()
            .find(|line| line.contains("DOWNLOAD") && line.contains("UPLOAD"))
            .unwrap();
        assert!(metrics.find("UPLOAD").unwrap() - metrics.find("DOWNLOAD").unwrap() <= 40);
        assert_eq!(app.activity, None);
    }
}

#[test]
fn appearance_edits_are_offline_and_do_not_change_measurement_or_export_options() {
    use super::theme::Palette;
    let mut app = app();
    app.push(Screen::Settings);
    let before = format!("{:?}", app.options);
    app.page_mut().selected = 8;
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.palette, Palette::Graphite);
    key(&mut app, KeyCode::Char('-'));
    assert_eq!(app.palette, Palette::Terminal);
    for _ in 0..4 {
        key(&mut app, KeyCode::Enter);
    }
    assert_eq!(app.palette, Palette::Terminal);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert!(app.compact);
    key(&mut app, KeyCode::Char('-'));
    assert!(!app.compact);
    assert_eq!(format!("{:?}", app.options), before);
    assert_eq!(app.activity, None);
}

#[test]
fn comfortable_metrics_are_large_and_compact_values_remain_exact() {
    let mut app = app();
    app.result = Some(result());
    app.push(Screen::Results);
    let text = render(&mut app, 120, 38).0;
    assert!(
        text.contains("███"),
        "five-row metric digits must be visible"
    );
    assert!(text.contains("Mbps"));
    assert!(
        text.contains("100.0 Mbps"),
        "large digits retain an exact text readout"
    );
    app.compact = true;
    let compact = render(&mut app, 120, 38).0;
    assert!(compact.contains("100.0 Mbps"));
    assert!(!compact.contains("███"));
    // A large but finite value that cannot fit in block digits must stay intact.
    app.compact = false;
    app.result.as_mut().unwrap().download.mbps = 1_000_000_000.0;
    assert!(render(&mut app, 120, 38).0.contains("1000000000.0 Mbps"));
}

#[test]
fn home_and_results_keep_exact_readings_at_every_layout_size() {
    let mut app = app();
    for language in [crate::i18n::Language::En, crate::i18n::Language::It] {
        app.language = language;
        for (download, upload) in [(455.5, 312.3), (1_000.0, 10_000.0)] {
            let mut saved = result();
            saved.download.mbps = download;
            saved.upload.mbps = upload;
            app.set_history(Ok(Archive::from_results(vec![saved.clone()])));
            app.result = Some(saved);
            for screen in [Screen::Home, Screen::Results] {
                app.pages.truncate(1);
                if screen != Screen::Home {
                    app.push(screen);
                }
                for (width, height) in [(80, 24), (120, 38), (214, 52)] {
                    let text = render(&mut app, width, height).0;
                    for value in [download, upload] {
                        assert!(
                            text.contains(&format!("{value:.1} Mbps")),
                            "{language:?} {screen:?} {width}x{height}: {text}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn short_summaries_keep_controls_nearby_and_long_results_remain_scrollable() {
    let mut app = app();
    let records = comparison_results();
    app.result = records.last().cloned();
    app.set_history(Ok(Archive::from_results(records)));
    for screen in [
        Screen::Results,
        Screen::Statistics,
        Screen::Compare,
        Screen::Live,
    ] {
        app.pages.truncate(1);
        app.push(screen);
        let text = render(&mut app, 214, 52).0;
        let footer = text.lines().position(|line| line.contains("quit")).unwrap();
        assert!(
            footer < 45,
            "{screen:?} leaves its controls across empty rows: {text}"
        );
        let labels: &[&str] = match screen {
            Screen::Results => &["DOWNLOAD", "UPLOAD", "IDLE LATENCY", "JITTER"],
            Screen::Statistics => &["MEDIAN DOWNLOAD", "MEDIAN UPLOAD", "MEDIAN LATENCY"],
            _ => &[],
        };
        if !labels.is_empty() {
            let header = text.lines().find(|line| line.contains(labels[0])).unwrap();
            for pair in labels.windows(2) {
                assert!(header.find(pair[1]).unwrap() - header.find(pair[0]).unwrap() <= 40);
            }
        }
    }
    app.pages.truncate(1);
    app.push(Screen::Results);
    app.result
        .as_mut()
        .unwrap()
        .analysis
        .as_mut()
        .unwrap()
        .quality
        .findings[0]
        .evidence = format!(
        "{} end-of-long-result",
        "Detailed finding evidence ".repeat(500)
    );
    app.page_mut().scroll = u16::MAX;
    let text = render(&mut app, 214, 52).0;
    assert!(text.contains("end-of-long-result"));
    assert!(text.contains("LOADED LATENCY"));
    assert!(text.contains("400.0 Mbps"));
    assert!(app.page().scroll > 0);
    assert!(text.lines().nth(50).unwrap().contains("quit"));
}

#[test]
fn sparse_history_keeps_details_near_rows_and_long_history_uses_the_viewport() {
    let mut app = app();
    let records = |count| {
        (0..count)
            .map(|i| {
                let mut saved = result();
                saved.timestamp += chrono::Duration::seconds(i);
                saved.backend = format!("row-{i:02}");
                saved
            })
            .collect()
    };
    app.set_history(Ok(Archive::from_results(records(8))));
    app.push(Screen::History);
    let text = render(&mut app, 214, 52).0;
    let last_row = text
        .lines()
        .position(|line| line.contains("row-00"))
        .unwrap();
    let status = text
        .lines()
        .position(|line| line.contains("Run 1 of 8"))
        .unwrap();
    assert!(status - last_row <= 2);
    assert_eq!(app.history_page_size, 8);
    app.set_history(Ok(Archive::from_results(records(40))));
    render(&mut app, 214, 52);
    assert!(app.history_page_size > 8);
    key(&mut app, KeyCode::End);
    let text = render(&mut app, 214, 52).0;
    assert!(text.contains("row-00"));
    assert!(text.contains("Run 40 of 40"));
    assert!(text.lines().nth(50).unwrap().contains("quit"));
}

#[test]
fn comparison_verdict_and_highlight_are_visible_without_scrolling_at_minimum_size() {
    for language in [crate::i18n::Language::En, crate::i18n::Language::It] {
        let mut app = app();
        app.language = language;
        app.set_history(Ok(Archive::from_results(comparison_results())));
        app.push(Screen::History);
        key(&mut app, KeyCode::Char('c'));
        let comparison = app.comparison.as_ref().unwrap();
        let messages = [&comparison.metrics.verdict, &comparison.metrics.highlight]
            .map(|message| crate::i18n::narrative(language, message));
        let compact = |value: &str| {
            value
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>()
        };
        let text = render(&mut app, 80, 24).0;
        for message in messages {
            assert!(
                compact(&text).contains(&compact(&message)),
                "{language:?}: {text}"
            );
        }
        assert_eq!(app.page().scroll, 0);
    }
}

#[test]
fn result_quality_label_and_owned_finding_titles_are_localized_before_composition() {
    for language in crate::i18n::Language::ALL {
        let mut app = app();
        app.language = language;
        let mut saved = comparison_results().pop().unwrap();
        let findings = &mut saved.analysis.as_mut().unwrap().quality.findings;
        findings.truncate(1);
        findings[0].title = "Connection looks healthy under this test".into();
        let source_title = findings[0].title.clone();
        app.result = Some(saved);
        app.push(Screen::Results);
        let rendered = render(&mut app, 214, 52).0;
        assert!(rendered.contains(&crate::i18n::text(language, "QUALITY")));
        assert!(
            rendered.contains(&crate::i18n::narrative(language, &source_title)),
            "{}: {rendered}",
            language.code()
        );
    }
}

#[test]
fn settings_descriptions_are_contextual_and_scroll_without_losing_selection() {
    let mut app = app();
    app.push(Screen::Settings);
    app.page_mut().selected = 8;
    let text = render(&mut app, 80, 24).0;
    assert!(text.contains("TERMINAL COLORS"));
    key(&mut app, KeyCode::PageDown);
    let text = render(&mut app, 80, 24).0;
    assert!(text.contains("profile"));
    assert_eq!(app.page().selected, 8);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.page().scroll, 0);
    let text = render(&mut app, 80, 24).0;
    assert!(text.contains("READABILITY"));
    key(&mut app, KeyCode::PageDown);
    assert!(render(&mut app, 80, 24).0.contains("font"));
}

#[test]
fn history_headers_and_numeric_cells_share_their_right_edge() {
    let mut app = app();
    app.set_history(Ok(Archive::from_results(vec![result()])));
    app.push(Screen::History);
    for (width, height) in [(80, 24), (180, 48)] {
        let (text, _) = render(&mut app, width, height);
        let header = text.lines().find(|line| line.contains("QUALITY")).unwrap();
        let row = text
            .lines()
            .find(|line| line.contains("01-01 00:00") && line.contains("fixture"))
            .unwrap();
        let end_of = |text: &str, needle: &str| {
            let prefix = &text[..text.find(needle).unwrap()];
            prefix.chars().count() + needle.chars().count()
        };
        assert_eq!(end_of(header, "QUALITY"), end_of(row, "n/a"));
        assert_eq!(end_of(header, "DOWN Mbps"), end_of(row, "100.0"));
    }
}

#[test]
fn optional_fixed_palettes_keep_normal_text_high_contrast() {
    use ratatui::style::Color;
    let luminance = |color| {
        let Color::Rgb(r, g, b) = color else {
            panic!("RGB fixture");
        };
        let linear = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    };
    for theme in [Theme::rgb(), Theme::light()] {
        for color in [
            theme.text,
            theme.muted,
            theme.focus,
            theme.success,
            theme.warning,
            theme.error,
        ] {
            let (a, b) = (luminance(color), luminance(theme.background));
            assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5, "{color:?}");
        }
    }
}

#[test]
fn capture_readability_frames_when_explicitly_requested() {
    let Ok(root) = std::env::var("READABILITY_SNAPSHOT_DIR") else {
        return;
    };
    std::fs::create_dir_all(&root).unwrap();
    let mut app = app();
    let mut latest = result();
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
    let samples = (0..8)
        .map(|i| {
            let mut sample = latest.clone();
            sample.timestamp -= chrono::Duration::hours(7 - i);
            sample.download.mbps =
                [512.0, 630.0, 605.0, 680.0, 580.0, 720.0, 694.0, 455.5][i as usize];
            sample
        })
        .collect();
    app.set_history(Ok(Archive::from_results(samples)));
    app.result = Some(latest.clone());
    app.save_notice = "SAVED to local history · example data".into();
    for (theme_name, theme) in [
        ("native", Theme::ansi()),
        ("graphite", Theme::rgb()),
        ("light", Theme::light()),
        ("mono", Theme::monochrome()),
    ] {
        for (name, screen) in [
            ("home", Screen::Home),
            ("results", Screen::Results),
            ("settings", Screen::Settings),
            ("history", Screen::History),
            ("statistics", Screen::Statistics),
            ("live", Screen::Live),
        ] {
            app.pages.truncate(1);
            app.palette = match theme_name {
                "graphite" => super::theme::Palette::Graphite,
                "light" => super::theme::Palette::Light,
                "mono" => super::theme::Palette::Monochrome,
                _ => super::theme::Palette::Terminal,
            };
            if matches!(screen, Screen::Live | Screen::Results) {
                app.push(Screen::Configure);
            }
            if screen != Screen::Home {
                app.push(screen);
            }
            app.activity = if screen == Screen::Live {
                Some(Activity::Test)
            } else {
                None
            };
            app.live.phase = crate::model::TestPhase::Download;
            app.live.download_mbps = Some(642.7);
            app.live.ping_ms = Some(10.0);
            app.live.jitter_ms = Some(2.0);
            app.live.speedometer.snap_to_with_peak(642.7, 700.0);
            for language in [crate::i18n::Language::En, crate::i18n::Language::It] {
                app.language = language;
                for (width, height) in [(80, 24), (120, 38), (214, 52)] {
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal
                        .draw(|frame| view::draw(frame, &mut app, theme, Duration::from_secs(6)))
                        .unwrap();
                    let buffer = terminal.backend().buffer();
                    let text = (0..height)
                        .map(|y| {
                            (0..width)
                                .map(|x| buffer[(x, y)].symbol())
                                .collect::<String>()
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    save_frame(
                        &root,
                        &format!("{theme_name}-{name}-{}-{width}x{height}", language.code()),
                        &text,
                        buffer,
                    );
                }
            }
            if app.pages.len() > 1 {
                app.pages.pop();
            }
        }
    }
}

#[test]
fn wide_save_failures_remain_scrollable_without_displacing_the_quality_summary() {
    let mut app = app();
    app.result = Some(result());
    app.save_notice = format!(
        "SAVE FAILED · {} end-of-save-error",
        "path context ".repeat(300)
    );
    app.push(Screen::Results);
    let text = render(&mut app, 120, 38).0;
    assert!(text.contains("See details"));
    assert!(text.contains("LOADED LATENCY"));
    app.page_mut().scroll = u16::MAX;
    assert!(render(&mut app, 120, 38).0.contains("end-of-save-error"));
}

#[test]
fn eight_languages_render_every_screen_and_selected_tab_at_minimum_size() {
    use crate::i18n::{text, Language};
    for language in Language::ALL {
        let mut app = app();
        app.language = language;
        app.set_history(Ok(Archive::from_results(vec![result(), result()])));
        app.result = Some(result());
        app.tool = Some(Tool::DnsList);
        app.report = Some(Load::Ready("raw vendor output /CASE/{0}".into()));
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
            for (w, h) in [(80, 24), (120, 38), (79, 23), (1, 1)] {
                let (buffer_text, _) = render(&mut app, w, h);
                if w >= 80 {
                    assert!(
                        buffer_text.contains("SPEEDTEST"),
                        "{} {screen:?}",
                        language.code()
                    );
                    assert!(!buffer_text.contains("LAST RESULT AVAILABLE"));
                    // The active tab, not only the beginning of a long translated
                    // tab strip, must stay on screen after terminal font zoom.
                    if screen == Screen::Settings {
                        assert!(
                            buffer_text.contains(&format!("> {}", text(language, "Settings"))),
                            "{}: {buffer_text}",
                            language.code()
                        );
                    }
                }
            }
            app.pages.pop();
        }
    }
}

#[test]
fn language_switch_is_presentation_only_and_font_guide_scroll_is_modal() {
    use crate::i18n::Language;
    let mut app = app();
    app.push(Screen::Settings);
    app.pages.last_mut().unwrap().selected = 10;
    let options = format!("{:?}", app.options);
    for expected in [
        Language::It,
        Language::Es,
        Language::Fr,
        Language::De,
        Language::Pt,
        Language::ZhCn,
        Language::Ja,
        Language::En,
    ] {
        assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
        assert_eq!(app.language, expected);
        assert_eq!(format!("{:?}", app.options), options);
        assert_eq!(app.activity, None);
    }
    assert_eq!(key(&mut app, KeyCode::Char('z')), Effect::None);
    assert_eq!(app.modal, Some(Modal::TextSize));
    key(&mut app, KeyCode::PageDown);
    assert!(app.modal_scroll > 0);
    render(&mut app, 80, 24);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, None);
    assert_eq!(app.page().selected, 10);
    assert_eq!(app.screen(), Screen::Settings);
    assert_eq!(app.activity, None);
}

#[test]
fn capture_localized_frames_when_requested() {
    let Ok(root) = std::env::var("LOCALIZED_SNAPSHOT_DIR") else {
        return;
    };
    std::fs::create_dir_all(&root).unwrap();
    for language in crate::i18n::Language::ALL {
        let mut app = app();
        app.language = language;
        app.set_history(Ok(Archive::from_results(vec![])));
        for (name, screen) in [("home", Screen::Home), ("settings", Screen::Settings)] {
            app.push(screen);
            for (w, h) in [(80, 24), (120, 38)] {
                let (text, buffer) = render(&mut app, w, h);
                save_frame(
                    &root,
                    &format!("{}-{name}-{w}", language.code()),
                    &text,
                    &buffer,
                );
            }
            app.pages.pop();
        }
    }
}

#[test]
fn cancel_dialog_does_not_inherit_scrolled_help_position() {
    let mut app = app();
    start(&mut app);
    key(&mut app, KeyCode::Char('z'));
    key(&mut app, KeyCode::PageDown);
    render(&mut app, 80, 24);
    assert!(app.modal_scroll > 0);
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.modal_scroll, 0);
    assert!(render(&mut app, 80, 24)
        .0
        .contains("Cancel the active task?"));
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.activity, Some(Activity::Test));
}
