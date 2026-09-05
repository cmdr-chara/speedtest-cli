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
    // The new workspace has a deliberate height cap. A larger terminal exposes
    // more lines but must not pretend the remaining overflow no longer exists.
    assert!(app.page().scroll < small_scroll);
    assert!(text.contains("PgUp/PgDn"));
    let capped_scroll = app.page().scroll;
    render(&mut app, 100, 100);
    assert_eq!(app.page().scroll, capped_scroll);
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
fn workspace_caps_width_and_height_without_changing_terminal_dimensions() {
    use ratatui::layout::Rect;
    assert_eq!(
        view::workspace(Rect::new(0, 0, 80, 24)),
        Rect::new(0, 0, 80, 24)
    );
    assert_eq!(
        view::workspace(Rect::new(10, 20, 180, 48)),
        Rect::new(40, 25, 120, 38)
    );
    assert_eq!(view::workspace(Rect::new(0, 0, 300, 100)).width, 120);
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
            .find(|line| line.contains("01-01 00:00 UTC"))
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
    latest.download.mbps = 742.8;
    latest.upload.mbps = 128.4;
    latest.download.bytes = 92_850_000;
    latest.upload.bytes = 16_050_000;
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
                [512.0, 630.0, 605.0, 680.0, 580.0, 720.0, 694.0, 742.8][i as usize];
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
            let mut terminal = Terminal::new(TestBackend::new(120, 38)).unwrap();
            terminal
                .draw(|frame| view::draw(frame, &mut app, theme, Duration::from_secs(6)))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let text = (0..38)
                .map(|y| {
                    (0..120)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            save_frame(&root, &format!("{theme_name}-{name}"), &text, buffer);
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
