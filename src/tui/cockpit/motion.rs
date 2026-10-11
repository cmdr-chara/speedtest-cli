//! Presentation-only choreography. Time is supplied by the shell, never by widgets.
//! Effects have bounded lifetimes; an expired effect still requests its final frame.
use std::time::Duration;

use ratatui::{buffer::Buffer, layout::Rect, style::Modifier};

use super::{state::Screen, theme::Theme};
use crate::{insights::HistoryMetric, model::TestPhase};

const ARRIVAL: f64 = 0.72;
const FOCUS: f64 = 0.24;
const PHASE: f64 = 0.48;
const MODAL: f64 = 0.28;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Scene {
    pub screen: Screen,
    pub selected: usize,
    pub phase: TestPhase,
    pub metric: HistoryMetric,
    pub revision: u64,
    pub modal: u8,
    pub size: (u16, u16),
    pub live: bool,
}

#[derive(Debug, Default)]
pub(super) struct Motion {
    scene: Option<Scene>,
    now: Duration,
    enabled: bool,
    arrival: Option<Duration>,
    focus: Option<Duration>,
    phase: Option<Duration>,
    modal: Option<Duration>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Frame {
    pub arrival: f64,
    pub focus: f64,
    pub phase: f64,
    pub modal: f64,
    pub seconds: Option<f64>,
}

impl Motion {
    pub fn observe(&mut self, scene: Scene, now: Duration, reduced: bool) {
        self.now = now;
        let previous = self.scene.replace(scene);
        self.enabled = !reduced && scene.size.0 >= 80 && scene.size.1 >= 24;
        if !self.enabled || previous.is_some_and(|old| old.size != scene.size) {
            self.settle();
            return;
        }
        if previous.is_none_or(|old| old.screen != scene.screen)
            || (scene.screen == Screen::Statistics
                && previous.is_some_and(|old| {
                    old.metric != scene.metric || old.revision != scene.revision
                }))
        {
            self.arrival = Some(now);
        }
        if previous.is_some_and(|old| old.selected != scene.selected) {
            self.focus = Some(now);
        }
        if previous.is_some_and(|old| old.phase != scene.phase) {
            self.phase = Some(now);
        }
        if scene.modal == 0 {
            self.modal = None;
        } else if previous.is_none_or(|old| old.modal != scene.modal) {
            self.modal = Some(now);
        }
    }

    pub fn advance(&mut self, now: Duration) {
        self.now = now;
    }

    pub fn frame(&self) -> Frame {
        Frame {
            arrival: progress(self.now, self.arrival, ARRIVAL),
            focus: progress(self.now, self.focus, FOCUS),
            phase: progress(self.now, self.phase, PHASE),
            modal: progress(self.now, self.modal, MODAL),
            seconds: self.enabled.then_some(self.now.as_secs_f64()),
        }
    }

    pub fn needs_frame(&self) -> bool {
        self.enabled
            && (self.arrival.is_some()
                || self.focus.is_some()
                || self.phase.is_some()
                || self.modal.is_some()
                || self.scene.is_some_and(|s| s.live && s.modal == 0))
    }

    pub fn finish_frame(&mut self) {
        for (start, duration) in [
            (&mut self.arrival, ARRIVAL),
            (&mut self.focus, FOCUS),
            (&mut self.phase, PHASE),
            (&mut self.modal, MODAL),
        ] {
            if progress(self.now, *start, duration) >= 1.0 {
                *start = None;
            }
        }
    }

    fn settle(&mut self) {
        self.arrival = None;
        self.focus = None;
        self.phase = None;
        self.modal = None;
    }
}

fn progress(now: Duration, start: Option<Duration>, duration: f64) -> f64 {
    start.map_or(1.0, |start| {
        (now.saturating_sub(start).as_secs_f64() / duration).clamp(0.0, 1.0)
    })
}

pub(super) fn ease(value: f64) -> f64 {
    1.0 - (1.0 - value.clamp(0.0, 1.0)).powi(3)
}

/// A diagonal light sweep changes emphasis, never text, numbers or glyph widths.
/// Native and monochrome palettes remain native; no guessed background blending.
pub(super) fn illuminate(buffer: &mut Buffer, area: Rect, theme: Theme, progress: f64) {
    if progress >= 1.0 || area.is_empty() {
        return;
    }
    let area = area.intersection(buffer.area);
    let reach = f64::from(area.width) + f64::from(area.height) * 1.6;
    let head = ease(progress) * (reach + 12.0) - 6.0;
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let distance = (f64::from(x - area.x) + f64::from(y - area.y) * 1.6 - head).abs();
            if distance > 4.0 {
                continue;
            }
            let cell = &mut buffer[(x, y)];
            if cell.symbol().trim().is_empty() || cell.modifier.contains(Modifier::REVERSED) {
                continue;
            }
            // Error/success colors retain their meaning throughout the transition.
            if [theme.text, theme.focus, theme.muted, theme.line].contains(&cell.fg) {
                cell.set_fg(theme.focus);
                cell.set_style(ratatui::style::Style::default().add_modifier(Modifier::BOLD));
            }
        }
    }
}

/// Resolve the decorative, fixed-width block logo through a narrow scan front.
/// This is intentionally limited to the brand, never labels or measured values.
pub(super) fn assemble_brand(buffer: &mut Buffer, area: Rect, theme: Theme, progress: f64) {
    if progress >= 1.0 {
        return;
    }
    let area = area.intersection(buffer.area);
    let front = ease(progress) * f64::from(area.width + 5);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let cell = &mut buffer[(x, y)];
            if cell.symbol().trim().is_empty() {
                continue;
            }
            let ahead = f64::from(x - area.x) - front;
            if ahead > 0.0 {
                cell.set_symbol(if ahead < 3.0 { "▓" } else { "░" });
                cell.set_fg(theme.line);
            } else if ahead > -3.0 {
                cell.set_fg(theme.text);
            }
        }
    }
}

pub(super) fn focus_selection(buffer: &mut Buffer, area: Rect, progress: f64) {
    if progress >= 1.0 {
        return;
    }
    let area = area.intersection(buffer.area);
    for y in area.y..area.bottom() {
        let first =
            (area.x..area.right()).find(|x| buffer[(*x, y)].modifier.contains(Modifier::REVERSED));
        let Some(first) = first else {
            continue;
        };
        let last = (first..area.right())
            .take_while(|x| buffer[(*x, y)].modifier.contains(Modifier::REVERSED))
            .last()
            .unwrap_or(first);
        let head = first + (ease(progress) * f64::from(last - first)) as u16;
        for x in head.saturating_sub(2).max(first)..=head {
            buffer[(x, y)]
                .set_style(ratatui::style::Style::default().add_modifier(Modifier::UNDERLINED));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> Scene {
        Scene {
            screen: Screen::Home,
            selected: 0,
            phase: TestPhase::Preparing,
            metric: HistoryMetric::Download,
            revision: 0,
            modal: 0,
            size: (120, 38),
            live: false,
        }
    }

    #[test]
    fn arrival_requests_a_final_settled_frame_then_stops() {
        let mut motion = Motion::default();
        motion.observe(scene(), Duration::ZERO, false);
        assert!(motion.needs_frame());
        motion.advance(Duration::from_secs(2));
        assert_eq!(motion.frame().arrival, 1.0);
        assert!(motion.needs_frame());
        motion.finish_frame();
        assert!(!motion.needs_frame());
    }

    #[test]
    fn reduced_motion_resize_and_hidden_live_stop_effects() {
        let mut motion = Motion::default();
        let mut scene = scene();
        scene.live = true;
        motion.observe(scene, Duration::ZERO, false);
        motion.observe(scene, Duration::from_millis(100), true);
        assert!(!motion.needs_frame());
        assert!(motion.frame().seconds.is_none());
        assert_eq!(motion.frame().arrival, 1.0);
        scene.size = (40, 12);
        motion.observe(scene, Duration::from_millis(200), false);
        assert!(!motion.needs_frame());
        scene.size = (120, 38);
        scene.modal = 1;
        motion.observe(scene, Duration::from_millis(300), false);
        assert!(!motion.needs_frame());
    }

    #[test]
    fn navigation_and_chart_revision_restart_but_samples_do_not() {
        let mut motion = Motion::default();
        let mut scene = scene();
        motion.observe(scene, Duration::ZERO, false);
        motion.observe(scene, Duration::from_millis(400), false);
        assert!(motion.frame().arrival > 0.5);
        scene.screen = Screen::Statistics;
        motion.observe(scene, Duration::from_secs(1), false);
        assert_eq!(motion.frame().arrival, 0.0);
        scene.revision += 1;
        motion.observe(scene, Duration::from_secs(2), false);
        assert_eq!(motion.frame().arrival, 0.0);
        scene.metric = HistoryMetric::Upload;
        motion.observe(scene, Duration::from_secs(3), false);
        assert_eq!(motion.frame().arrival, 0.0);
    }

    #[test]
    fn effects_preserve_glyphs_and_terminal_color_contract() {
        for theme in [Theme::ansi(), Theme::monochrome()] {
            let mut buffer = Buffer::empty(Rect::new(0, 0, 30, 4));
            buffer.set_string(0, 1, "速度 455.5 Mbps", theme.base());
            let before: Vec<_> = buffer
                .content
                .iter()
                .map(|c| c.symbol().to_string())
                .collect();
            illuminate(&mut buffer, Rect::new(0, 0, 30, 4), theme, 0.1);
            assert_eq!(
                before,
                buffer
                    .content
                    .iter()
                    .map(|c| c.symbol().to_string())
                    .collect::<Vec<_>>()
            );
            assert!(buffer.content.iter().all(|c| !matches!(
                c.fg,
                ratatui::style::Color::Rgb(..) | ratatui::style::Color::Indexed(_)
            )));
        }
    }
}
