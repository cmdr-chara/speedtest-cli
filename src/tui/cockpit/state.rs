//! Navigation and transitions. No terminal, filesystem, process, or network I/O.
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::TableState;

use crate::{
    cli::InternetBackendArg, engine::EngineEvent, model::TestResult, session::TestOptions,
};

use super::{
    services::{Archive, Tool},
    theme::Palette,
};
use crate::tui::app::App;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Screen {
    Home,
    Configure,
    Live,
    Results,
    History,
    Statistics,
    Compare,
    Dns,
    Diagnostics,
    Settings,
    Tool,
    Failure,
}

pub(super) const SECTIONS: [Screen; 7] = [
    Screen::Home,
    Screen::Configure,
    Screen::History,
    Screen::Statistics,
    Screen::Dns,
    Screen::Diagnostics,
    Screen::Settings,
];
pub(super) const HOME: [Screen; 6] = [
    Screen::Configure,
    Screen::History,
    Screen::Statistics,
    Screen::Dns,
    Screen::Diagnostics,
    Screen::Settings,
];

impl Screen {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Configure => "Test",
            Self::Live => "Live test",
            Self::Results => "Results",
            Self::History => "History",
            Self::Statistics => "Statistics",
            Self::Compare => "Compare",
            Self::Dns => "DNS Tools",
            Self::Diagnostics => "Diagnostics",
            Self::Settings => "Settings",
            Self::Tool => "Diagnostic report",
            Self::Failure => "Test interrupted",
        }
    }
}

#[derive(Debug)]
pub(super) struct Page {
    pub screen: Screen,
    pub selected: usize,
    pub scroll: u16,
}

impl Page {
    fn new(screen: Screen) -> Self {
        Self {
            screen,
            selected: 0,
            scroll: 0,
        }
    }
}

#[derive(Debug)]
pub(super) enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Activity {
    Test,
    Saving,
    Tool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Modal {
    Help,
    TextSize,
    Cancel { quit: bool, confirm: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Effect {
    None,
    StartTest,
    LoadHistory,
    StartTool(Tool),
    Cancel,
    Quit,
    Interrupt,
}

#[derive(Debug)]
pub(super) struct Cockpit {
    pub pages: Vec<Page>,
    pub options: TestOptions,
    pub reduced_motion: bool,
    pub palette: Palette,
    pub compact: bool,
    pub language: crate::i18n::Language,
    pub modal_scroll: u16,
    pub history: Load<Archive>,
    pub table: TableState,
    pub live: App,
    pub result: Option<TestResult>,
    pub recent: Option<TestResult>,
    pub activity: Option<Activity>,
    pub modal: Option<Modal>,
    pub tool: Option<Tool>,
    pub report: Option<Load<String>>,
    pub failure: String,
    pub notice: String,
    pub save_notice: String,
    pub quit_after_save: bool,
}

impl Cockpit {
    pub fn new(options: TestOptions) -> Self {
        Self {
            pages: vec![Page::new(Screen::Home)],
            options,
            reduced_motion: false,
            palette: Palette::default(),
            compact: false,
            language: crate::i18n::cli_language(),
            modal_scroll: 0,
            history: Load::Loading,
            table: TableState::default(),
            live: App::default(),
            result: None,
            recent: None,
            activity: None,
            modal: None,
            tool: None,
            report: None,
            failure: String::new(),
            notice: String::new(),
            save_notice: String::new(),
            quit_after_save: false,
        }
    }

    pub fn page(&self) -> &Page {
        self.pages.last().expect("home is never popped")
    }
    pub fn page_mut(&mut self) -> &mut Page {
        self.pages.last_mut().expect("home is never popped")
    }
    pub fn screen(&self) -> Screen {
        self.page().screen
    }
    pub fn push(&mut self, screen: Screen) {
        self.pages.push(Page::new(screen));
    }
    fn replace(&mut self, screen: Screen) {
        *self.page_mut() = Page::new(screen);
    }

    pub fn latest(&self) -> Option<&TestResult> {
        self.recent.as_ref().or_else(|| match &self.history {
            Load::Ready(archive) => archive.results.last(),
            _ => None,
        })
    }

    pub fn apply_engine(&mut self, event: EngineEvent) {
        if self.activity != Some(Activity::Test) {
            return;
        }
        self.live.apply(event);
        if self.reduced_motion {
            self.live.speedometer.snap_to_with_peak(
                self.live
                    .download_mbps
                    .filter(|_| self.live.phase == crate::model::TestPhase::Download)
                    .or(self.live.upload_mbps)
                    .unwrap_or(0.0),
                self.live.speedometer.peak_mbps(),
            );
        }
    }

    pub fn measured(&mut self, result: Result<TestResult, String>) {
        // Terminal engine events are not completion authority: only the returned result is.
        if self.activity != Some(Activity::Test) {
            return;
        }
        self.modal = self
            .modal
            .filter(|m| matches!(m, Modal::Help | Modal::TextSize));
        match result {
            Ok(result) => {
                self.live.apply(EngineEvent::Complete(result.clone()));
                self.result = Some(result);
                self.activity = Some(Activity::Saving);
                self.notice =
                    "Finishing completed result; please do not close the terminal.".into();
            }
            Err(error) => {
                self.activity = None;
                self.failure = error;
                self.notice.clear();
                self.replace(Screen::Failure);
            }
        }
    }

    pub fn saved(&mut self, outcome: Result<(), String>) {
        if self.activity != Some(Activity::Saving) {
            return;
        }
        self.activity = None;
        self.recent = self.result.clone();
        self.save_notice = match outcome {
            Ok(()) if !self.options.no_save => "SAVED to local history".into(),
            Ok(()) if self.options.output.is_some() => "EXPORTED • automatic history is off".into(),
            Ok(()) => "NOT SAVED • session result only".into(),
            Err(error) => format!("SAVE FAILED • result retained in this screen: {error}"),
        };
        self.notice.clear();
        self.replace(Screen::Results);
    }

    pub fn set_history(&mut self, outcome: Result<Archive, String>) {
        self.history = match outcome {
            Ok(value) => Load::Ready(value),
            Err(error) => Load::Failed(error),
        };
        let count = self.history_count();
        for page in &mut self.pages {
            if page.screen == Screen::History {
                page.selected = page.selected.min(count.saturating_sub(1));
            }
        }
    }

    pub fn tool_finished(&mut self, result: Result<String, String>) {
        if self.activity != Some(Activity::Tool) {
            return;
        }
        self.activity = None;
        self.modal = self
            .modal
            .filter(|m| matches!(m, Modal::Help | Modal::TextSize));
        self.report = Some(match result {
            Ok(text) => Load::Ready(text),
            Err(error) => Load::Failed(error),
        });
        self.page_mut().scroll = 0;
    }

    fn history_count(&self) -> usize {
        match &self.history {
            Load::Ready(archive) => archive.results.len(),
            _ => 0,
        }
    }

    fn select_count(&self) -> usize {
        match self.screen() {
            Screen::Home => HOME.len(),
            Screen::Configure => 12,
            Screen::Settings => 11,
            Screen::History => self.history_count(),
            Screen::Dns => Tool::DNS.len(),
            Screen::Diagnostics => Tool::DIAGNOSTICS.len(),
            _ => 0,
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let count = self.select_count();
        let page = self.page_mut();
        if count > 0 {
            page.scroll = 0;
            page.selected = (page.selected as isize + delta).rem_euclid(count as isize) as usize;
        } else {
            page.scroll = page
                .scroll
                .saturating_add_signed(delta.clamp(-100, 100) as i16);
        }
    }

    fn sibling(&mut self, delta: isize) {
        let section = self
            .pages
            .iter()
            .rev()
            .find_map(|page| SECTIONS.iter().position(|s| *s == page.screen))
            .unwrap_or(0);
        let next =
            SECTIONS[(section as isize + delta).rem_euclid(SECTIONS.len() as isize) as usize];
        // Sibling switches replace the section, rather than making Back walk a tab history.
        self.pages.truncate(1);
        if next != Screen::Home {
            self.push(next);
        }
    }

    fn back(&mut self) {
        if self.pages.len() > 1 {
            self.pages.pop();
        }
        self.notice.clear();
    }

    fn begin_test(&mut self) -> Effect {
        if self.screen() == Screen::Failure {
            self.replace(Screen::Live);
        } else {
            self.push(Screen::Live);
        }
        self.live = App::default();
        self.result = None;
        self.failure.clear();
        self.notice.clear();
        self.activity = Some(Activity::Test);
        Effect::StartTest
    }

    fn edit(&mut self, forward: bool) {
        let selected = self.page().selected;
        let index = if self.screen() == Screen::Configure {
            let Some(index) = selected.checked_sub(1) else {
                return;
            };
            index
        } else {
            selected
        };
        let delta = if forward { 1i64 } else { -1 };
        match index {
            0 => {
                self.options.backend = match self.options.backend {
                    InternetBackendArg::Cloudflare => InternetBackendArg::Librespeed,
                    InternetBackendArg::Librespeed => InternetBackendArg::Cloudflare,
                }
            }
            1 => self.options.duration = (self.options.duration as i64 + delta).clamp(3, 30) as u64,
            2 => {
                self.options.streams = (i64::from(self.options.streams) + delta).clamp(1, 16) as u8
            }
            3 => {
                self.options.fps = (i64::from(self.options.fps) + delta * 30).clamp(30, 240) as u16
            }
            4 => self.options.no_save = !self.options.no_save,
            5 => {
                self.options.timeout =
                    (self.options.timeout as i64 + delta * 10).clamp(1, 600) as u64
            }
            6 => self.reduced_motion = !self.reduced_motion,
            7 => {
                self.options.duration = 8;
                self.options.streams = 2;
                self.options.fps = 60;
                self.options.timeout = 120;
            }
            8 => self.palette = self.palette.cycle(forward),
            9 => self.compact = !self.compact,
            10 => self.language = self.language.cycle(forward),
            _ => {}
        }
        self.notice = "Session settings updated. Nothing is written until a test completes.".into();
    }

    /// Hidden controls must not start work while the terminal is too small.
    pub fn key_at_size(&mut self, key: KeyEvent, width: u16, height: u16) -> Effect {
        if (width < 80 || height < 24)
            && self.modal.is_none()
            && !matches!(
                key.code,
                KeyCode::Esc
                    | KeyCode::Backspace
                    | KeyCode::Char('q')
                    | KeyCode::Char('?')
                    | KeyCode::Char('z')
            )
            && !(key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            return Effect::None;
        }
        self.key(key)
    }

    pub fn key(&mut self, key: KeyEvent) -> Effect {
        if key.kind == KeyEventKind::Release {
            return Effect::None;
        }
        if key.kind == KeyEventKind::Repeat
            && !matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Char('j')
                    | KeyCode::Char('k')
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Char('+')
                    | KeyCode::Char('-')
            )
        {
            return Effect::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if self.activity == Some(Activity::Saving) {
                self.quit_after_save = true;
                self.notice = "Finishing the save, then exiting.".into();
                return Effect::None;
            }
            return Effect::Interrupt;
        }
        if let Some(modal) = self.modal {
            match modal {
                Modal::Help | Modal::TextSize => {
                    match key.code {
                        KeyCode::Down | KeyCode::Char('j') => {
                            self.modal_scroll = self.modal_scroll.saturating_add(1)
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            self.modal_scroll = self.modal_scroll.saturating_sub(1)
                        }
                        KeyCode::PageDown => {
                            self.modal_scroll = self.modal_scroll.saturating_add(8)
                        }
                        KeyCode::PageUp => self.modal_scroll = self.modal_scroll.saturating_sub(8),
                        _ => {}
                    }
                    if matches!(
                        key.code,
                        KeyCode::Esc
                            | KeyCode::Backspace
                            | KeyCode::Char('?')
                            | KeyCode::Enter
                            | KeyCode::Char('q')
                    ) {
                        self.modal = None;
                    }
                }
                Modal::Cancel { quit, mut confirm } => {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => confirm = true,
                        KeyCode::Left
                        | KeyCode::Right
                        | KeyCode::Tab
                        | KeyCode::BackTab
                        | KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::Char('j')
                        | KeyCode::Char('k') => {
                            self.modal = Some(Modal::Cancel {
                                quit,
                                confirm: !confirm,
                            });
                            return Effect::None;
                        }
                        KeyCode::Esc
                        | KeyCode::Backspace
                        | KeyCode::Char('n')
                        | KeyCode::Char('q') => {
                            self.modal = None;
                            return Effect::None;
                        }
                        KeyCode::Enter => {}
                        _ => return Effect::None,
                    }
                    self.modal = None;
                    if confirm {
                        self.activity = None;
                        self.report = None;
                        self.back();
                        self.notice = "CANCELLED • no incomplete result was saved.".into();
                        return if quit { Effect::Quit } else { Effect::Cancel };
                    }
                }
            }
            return Effect::None;
        }
        if key.code == KeyCode::Char('z') {
            self.modal_scroll = 0;
            self.modal = Some(Modal::TextSize);
            return Effect::None;
        }
        if key.code == KeyCode::Char('?') {
            self.modal_scroll = 0;
            self.modal = Some(Modal::Help);
            return Effect::None;
        }
        if self.activity.is_some() {
            if self.activity == Some(Activity::Saving) {
                if key.code == KeyCode::Char('q') {
                    self.quit_after_save = true;
                }
            } else if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('q')
            ) {
                self.modal_scroll = 0;
                self.modal = Some(Modal::Cancel {
                    quit: key.code == KeyCode::Char('q'),
                    confirm: false,
                });
            }
            return Effect::None;
        }
        match key.code {
            KeyCode::Char('q') => return Effect::Quit,
            KeyCode::Esc | KeyCode::Backspace => self.back(),
            KeyCode::Tab | KeyCode::Right => self.sibling(1),
            KeyCode::BackTab | KeyCode::Left => self.sibling(-1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::PageDown => self.page_mut().scroll = self.page().scroll.saturating_add(10),
            KeyCode::PageUp => self.page_mut().scroll = self.page().scroll.saturating_sub(10),
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char(' ')
                if matches!(self.screen(), Screen::Configure | Screen::Settings) =>
            {
                self.edit(true)
            }
            KeyCode::Char('-') if matches!(self.screen(), Screen::Configure | Screen::Settings) => {
                self.edit(false)
            }
            KeyCode::Char('r')
                if matches!(
                    self.screen(),
                    Screen::Home | Screen::History | Screen::Statistics | Screen::Compare
                ) =>
            {
                if !matches!(self.history, Load::Loading) {
                    self.history = Load::Loading;
                    return Effect::LoadHistory;
                }
            }
            KeyCode::Char('v') if self.screen() == Screen::Home => {
                if let Some(result) = self.latest().cloned() {
                    self.result = Some(result);
                    self.save_notice = "Most recent completed result".into();
                    self.push(Screen::Results);
                }
            }
            KeyCode::Char('c') if self.screen() == Screen::History => self.push(Screen::Compare),
            KeyCode::Char('r') if !matches!(self.screen(), Screen::Tool | Screen::Failure) => {}
            KeyCode::Enter | KeyCode::Char('r') => match self.screen() {
                Screen::Home => self.push(HOME[self.page().selected]),
                Screen::Configure if self.page().selected == 0 => return self.begin_test(),
                Screen::Configure | Screen::Settings => self.edit(true),
                Screen::Results => {
                    self.pages.truncate(1);
                    self.push(Screen::Configure);
                }
                Screen::History => {
                    if let Load::Ready(archive) = &self.history {
                        if let Some(result) = archive
                            .results
                            .iter()
                            .rev()
                            .nth(self.page().selected)
                            .cloned()
                        {
                            self.result = Some(result);
                            self.save_notice = "SAVED RESULT • local history".into();
                            self.push(Screen::Results);
                        }
                    }
                }
                Screen::Statistics => self.push(Screen::Compare),
                Screen::Dns | Screen::Diagnostics => {
                    let tools = if self.screen() == Screen::Dns {
                        Tool::DNS
                    } else {
                        Tool::DIAGNOSTICS
                    };
                    self.tool = Some(tools[self.page().selected]);
                    self.report = None;
                    self.push(Screen::Tool);
                }
                Screen::Tool => {
                    if let Some(tool) = self.tool {
                        self.activity = Some(Activity::Tool);
                        self.report = Some(Load::Loading);
                        self.page_mut().scroll = 0;
                        return Effect::StartTool(tool);
                    }
                }
                Screen::Failure => return self.begin_test(),
                _ => {}
            },
            _ => {}
        }
        Effect::None
    }
}
