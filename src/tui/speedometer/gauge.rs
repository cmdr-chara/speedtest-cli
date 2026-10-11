use crate::i18n::ui;
use std::f64::consts::PI;
use std::sync::OnceLock;

use ratatui::{
    prelude::{Alignment, Color, Frame, Line, Modifier, Rect, Span, Style},
    symbols::Marker,
    widgets::{
        canvas::{Canvas, Line as CanvasLine, Points},
        Block, Borders, Paragraph,
    },
};

use super::SpeedometerState;
use crate::tui::numerals;

/// Shell-supplied colors; legacy direct-run rendering retains its original palette.
#[derive(Debug, Clone, Copy)]
pub struct GaugePalette {
    pub background: Color,
    pub accent: Color,
    pub text: Color,
    pub secondary: Color,
    pub track: Color,
}

const START_ANGLE: f64 = PI * 1.15;
const SWEEP_ANGLE: f64 = PI * 1.30;
const TRACK_STEPS: usize = 260;
const TRACK_RADII: &[f64] = &[1.00, 0.975, 0.95];

#[derive(Clone, Copy, Default)]
struct GaugeEffects {
    enhanced: bool,
    seconds: Option<f64>,
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &SpeedometerState,
    accent: Color,
    show_value: bool,
) {
    render_with_background(frame, area, state, accent, show_value, Color::Black);
}

/// Embed the existing gauge in a shell without painting a black canvas over its theme.
pub fn render_with_background(
    frame: &mut Frame,
    area: Rect,
    state: &SpeedometerState,
    accent: Color,
    show_value: bool,
    background: Color,
) {
    render_themed(
        frame,
        area,
        state,
        show_value,
        GaugePalette {
            background,
            accent,
            text: Color::White,
            secondary: Color::Gray,
            track: Color::DarkGray,
        },
        false,
    );
}

pub fn render_themed(
    frame: &mut Frame,
    area: Rect,
    state: &SpeedometerState,
    show_value: bool,
    palette: GaugePalette,
    large_values: bool,
) {
    render_gauge(
        frame,
        area,
        state,
        show_value,
        palette,
        large_values,
        GaugeEffects::default(),
    );
}

/// Render the cockpit dial with orbital accents and a measured peak marker.
///
/// Time is presentation-only monotonic seconds. `None` disables every moving
/// decoration; neither path modifies measurement, the spring, or the dial scale.
pub fn render_animated(
    frame: &mut Frame,
    area: Rect,
    state: &SpeedometerState,
    show_value: bool,
    palette: GaugePalette,
    large_values: bool,
    animation_seconds: Option<f64>,
) {
    render_gauge(
        frame,
        area,
        state,
        show_value,
        palette,
        large_values,
        GaugeEffects {
            enhanced: true,
            seconds: animation_seconds.filter(|seconds| seconds.is_finite() && *seconds >= 0.0),
        },
    );
}

fn render_gauge(
    frame: &mut Frame,
    area: Rect,
    state: &SpeedometerState,
    show_value: bool,
    palette: GaugePalette,
    large_values: bool,
    effects: GaugeEffects,
) {
    let GaugePalette {
        background, accent, ..
    } = palette;
    if area.width < 40 || area.height < 8 {
        render_fallback(frame, area, state, palette, show_value);
        return;
    }

    let maximum = state.scale_mbps().max(1.0);
    let ratio = (state.displayed_mbps() / maximum).clamp(0.0, 1.0);
    let needle_angle = angle_for_ratio(ratio);

    let active_cap = point_on_arc(0.975, ratio);

    let labels = if area.width >= 62 && area.height >= 11 {
        scale_labels(maximum, area.width)
    } else {
        Vec::new()
    };

    let canvas = Canvas::default()
        .background_color(background)
        .block(Block::default().borders(Borders::NONE))
        .marker(Marker::Braille)
        .x_bounds([-1.28, 1.28])
        .y_bounds([-0.56, 1.18])
        .paint(move |ctx| {
            // The complete track is immutable geometry. Keep it off the frame
            // allocation/trigonometry path, especially during live measurements.
            for coords in track_layers() {
                ctx.draw(&Points {
                    coords,
                    color: palette.track,
                });
            }

            if effects.enhanced {
                draw_orbits(ctx, palette, effects.seconds);
                if show_value && effects.seconds.is_some() {
                    draw_needle_trail(ctx, state, ratio, maximum, palette);
                }
            }

            ctx.layer();

            if ratio > 0.0 {
                for radius in TRACK_RADII {
                    draw_arc(ctx, *radius, 0.0, ratio, accent);
                }
                if let Some(seconds) = effects.seconds.filter(|_| show_value) {
                    // The scan stays within the measured part of the scale. It
                    // never presents the decorative orbit as extra throughput.
                    let head = (seconds / 2.4).fract() * ratio;
                    draw_arc(ctx, 0.975, (head - 0.075).max(0.0), head, palette.text);
                }
                ctx.draw(&Points {
                    coords: &[active_cap],
                    color: palette.text,
                });
            }

            draw_ticks(ctx, ratio, palette);
            if effects.enhanced && show_value && state.peak_mbps() > 0.0 {
                draw_peak(ctx, (state.peak_mbps() / maximum).clamp(0.0, 1.0), palette);
            }
            draw_needle(ctx, needle_angle, palette);

            for (x, y, label) in &labels {
                ctx.print(
                    *x,
                    *y,
                    Line::from(Span::styled(
                        label.clone(),
                        Style::default().fg(palette.secondary),
                    )),
                );
            }
        });

    frame.render_widget(canvas, area);
    render_center_readout(frame, area, state, show_value, palette, large_values);
}

fn draw_ticks(ctx: &mut ratatui::widgets::canvas::Context<'_>, ratio: f64, palette: GaugePalette) {
    let accent = palette.accent;
    for index in 0..=20 {
        let fraction = index as f64 / 20.0;
        let angle = angle_for_ratio(fraction);
        let major = index % 5 == 0;
        let inner_radius = if major { 0.80 } else { 0.865 };
        let outer_radius = if major { 1.065 } else { 1.025 };
        let color = if fraction <= ratio {
            if major {
                accent
            } else {
                palette.secondary
            }
        } else if major {
            palette.secondary
        } else {
            palette.track
        };

        ctx.draw(&CanvasLine {
            x1: inner_radius * angle.cos(),
            y1: inner_radius * angle.sin(),
            x2: outer_radius * angle.cos(),
            y2: outer_radius * angle.sin(),
            color,
        });
    }
}

fn draw_orbits(
    ctx: &mut ratatui::widgets::canvas::Context<'_>,
    palette: GaugePalette,
    seconds: Option<f64>,
) {
    // Interrupted, thin arcs read as instrumentation, separate from the thick
    // quantitative track. Native/monochrome palettes remain entirely native.
    for index in 0..40 {
        let start = f64::from(index) / 40.0;
        draw_arc(ctx, 1.145, start, start + 0.009, palette.track);
    }
    for index in 0..24 {
        let start = f64::from(index) / 24.0;
        draw_arc(ctx, 0.70, start, start + 0.012, palette.track);
    }
    let Some(seconds) = seconds else {
        return;
    };

    // Opposing comet heads share one monotonic clock: no timers, randomness,
    // retained frame buffers, or changes to measured values are needed here.
    let outer_head = (seconds / 5.5).fract();
    let inner_head = 1.0 - (seconds / 7.0).fract();
    for (radius, head, direction) in [(1.145, outer_head, -1.0), (0.70, inner_head, 1.0)] {
        for (length, color) in [
            (0.095, palette.secondary),
            (0.045, palette.accent),
            (0.010, palette.text),
        ] {
            let tail = (head + direction * length).clamp(0.0, 1.0);
            draw_arc(ctx, radius, tail.min(head), tail.max(head), color);
        }
    }
}

fn draw_needle_trail(
    ctx: &mut ratatui::widgets::canvas::Context<'_>,
    state: &SpeedometerState,
    ratio: f64,
    maximum: f64,
    palette: GaugePalette,
) {
    let spread = (state.velocity.abs() / maximum * 0.07).min(0.12);
    if spread < 0.002 {
        return;
    }
    for (fraction, color) in [
        (1.0, palette.track),
        (0.6, palette.track),
        (0.25, palette.secondary),
    ] {
        let angle = angle_for_ratio(ratio - state.velocity.signum() * spread * fraction);
        ctx.draw(&CanvasLine {
            x1: 0.24 * angle.cos(),
            y1: 0.24 * angle.sin(),
            x2: 0.76 * angle.cos(),
            y2: 0.76 * angle.sin(),
            color,
        });
    }
}

fn draw_peak(
    ctx: &mut ratatui::widgets::canvas::Context<'_>,
    peak_ratio: f64,
    palette: GaugePalette,
) {
    let angle = angle_for_ratio(peak_ratio);
    let (x, y) = point_on_arc(0.91, peak_ratio);
    let radial = (angle.cos() * 0.035, angle.sin() * 0.035);
    let tangent = (-angle.sin() * 0.027, angle.cos() * 0.027);
    let corners = [
        (x + radial.0, y + radial.1),
        (x + tangent.0, y + tangent.1),
        (x - radial.0, y - radial.1),
        (x - tangent.0, y - tangent.1),
    ];
    for index in 0..corners.len() {
        let start = corners[index];
        let end = corners[(index + 1) % corners.len()];
        ctx.draw(&CanvasLine {
            x1: start.0,
            y1: start.1,
            x2: end.0,
            y2: end.1,
            color: palette.text,
        });
    }
}

fn draw_needle(
    ctx: &mut ratatui::widgets::canvas::Context<'_>,
    needle_angle: f64,
    palette: GaugePalette,
) {
    let accent = palette.accent;
    ctx.draw(&CanvasLine {
        x1: 0.0,
        y1: 0.0,
        x2: 0.82 * needle_angle.cos(),
        y2: 0.82 * needle_angle.sin(),
        color: palette.track,
    });

    for offset in [-0.008_f64, 0.0, 0.008] {
        let angle = needle_angle + offset;
        ctx.draw(&CanvasLine {
            x1: 0.0,
            y1: 0.0,
            x2: 0.77 * angle.cos(),
            y2: 0.77 * angle.sin(),
            color: accent,
        });
    }

    ctx.draw(&CanvasLine {
        x1: 0.0,
        y1: 0.0,
        x2: -0.13 * needle_angle.cos(),
        y2: -0.13 * needle_angle.sin(),
        color: palette.secondary,
    });

    let hub = [
        (0.0, 0.0),
        (0.018, 0.0),
        (-0.018, 0.0),
        (0.0, 0.018),
        (0.0, -0.018),
    ];
    ctx.draw(&Points {
        coords: &hub,
        color: accent,
    });
    ctx.draw(&Points {
        coords: &[(0.0, 0.0)],
        color: palette.text,
    });
}

fn render_center_readout(
    frame: &mut Frame,
    area: Rect,
    state: &SpeedometerState,
    show_value: bool,
    palette: GaugePalette,
    large_values: bool,
) {
    if area.width < 20 || area.height < 6 {
        return;
    }
    let large = large_values && area.height >= 16;
    let tall = large && area.height >= 23;
    let digit_height = if tall { 5 } else { 3 };
    let width = area.width.min(38);
    let x = area.x + (area.width - width) / 2;
    let y = area.y + area.height.saturating_mul(if large { 32 } else { 53 }) / 100;
    let height = area.bottom().saturating_sub(y).min(if tall {
        8
    } else if large {
        6
    } else {
        3
    });
    let value = if show_value {
        format!("{:.1}", state.displayed_mbps())
    } else {
        "—".into()
    };
    let base = Style::default().fg(palette.text).bg(palette.background);
    let big = large
        && height >= 5
        && numerals::draw(
            frame,
            Rect::new(x, y, width, digit_height),
            &value,
            base.add_modifier(Modifier::BOLD),
            Alignment::Center,
        );
    let mut lines = Vec::new();
    if big {
        lines.push(Line::from(vec![
            Span::styled(value, base.add_modifier(Modifier::BOLD)),
            Span::styled(ui(" Mbps"), base.fg(palette.secondary)),
        ]));
    } else {
        lines.push(Line::styled(value, base.add_modifier(Modifier::BOLD)));
        lines.push(Line::styled(ui("Mbps"), base.fg(palette.secondary)));
    }
    if show_value {
        lines.push(Line::styled(
            ui(format!(
                "peak {:.1}  •  scale {}",
                state.peak_mbps(),
                format_scale(state.scale_mbps())
            )),
            base.fg(palette.secondary),
        ));
    }
    let offset = if big { digit_height } else { 0 };
    frame.render_widget(
        Paragraph::new(lines)
            .style(base)
            .alignment(Alignment::Center),
        Rect::new(x, y + offset, width, height - offset),
    );
}

fn render_fallback(
    frame: &mut Frame,
    area: Rect,
    state: &SpeedometerState,
    palette: GaugePalette,
    show_value: bool,
) {
    let value = if show_value {
        format!("{:.1} Mbps", state.displayed_mbps())
    } else {
        "— Mbps".to_string()
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            value,
            Style::default()
                .fg(palette.accent)
                .bg(palette.background)
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center),
        area,
    );
}

fn track_layers() -> &'static [[(f64, f64); TRACK_STEPS + 1]; 3] {
    static TRACK: OnceLock<[[(f64, f64); TRACK_STEPS + 1]; 3]> = OnceLock::new();
    TRACK.get_or_init(|| {
        std::array::from_fn(|layer| {
            std::array::from_fn(|step| {
                point_on_arc(TRACK_RADII[layer], step as f64 / TRACK_STEPS as f64)
            })
        })
    })
}

fn draw_arc(
    ctx: &mut ratatui::widgets::canvas::Context<'_>,
    radius: f64,
    start: f64,
    end: f64,
    color: Color,
) {
    let start = start.clamp(0.0, 1.0);
    let end = end.clamp(start, 1.0);
    let steps = ((TRACK_STEPS as f64 * (end - start)).ceil() as usize).max(1);
    let mut coords = [(0.0, 0.0); TRACK_STEPS + 1];
    for (step, point) in coords.iter_mut().take(steps + 1).enumerate() {
        *point = point_on_arc(radius, start + (end - start) * step as f64 / steps as f64);
    }
    ctx.draw(&Points {
        coords: &coords[..=steps],
        color,
    });
}

fn point_on_arc(radius: f64, fraction: f64) -> (f64, f64) {
    let angle = angle_for_ratio(fraction.clamp(0.0, 1.0));
    (radius * angle.cos(), radius * angle.sin())
}

fn angle_for_ratio(ratio: f64) -> f64 {
    START_ANGLE - SWEEP_ANGLE * ratio.clamp(0.0, 1.0)
}

fn scale_labels(maximum: f64, area_width: u16) -> Vec<(f64, f64, String)> {
    [0.0_f64, 0.5, 1.0]
        .into_iter()
        .map(|fraction| {
            let label = format_tick(maximum * fraction);
            let angle = angle_for_ratio(fraction);
            let radius = if fraction == 0.5 { 1.10 } else { 1.13 };
            let char_width = 2.56 / f64::from(area_width.max(1));
            let x = radius * angle.cos() - label.chars().count() as f64 * char_width / 2.0;
            let y = radius * angle.sin();
            (x, y, label)
        })
        .collect()
}

fn format_tick(value: f64) -> String {
    if value >= 1_000.0 {
        let gigabits = value / 1_000.0;
        if gigabits.fract().abs() < f64::EPSILON {
            format!("{gigabits:.0}G")
        } else if (gigabits * 10.0).fract().abs() < f64::EPSILON {
            format!("{gigabits:.1}G")
        } else {
            format!("{gigabits:.2}G")
        }
    } else if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn format_scale(value: f64) -> String {
    if value >= 1_000.0 {
        let gigabits = value / 1_000.0;
        if gigabits.fract().abs() < f64::EPSILON {
            format!("{gigabits:.0} Gbps")
        } else {
            format!("{gigabits:.1} Gbps")
        }
    } else {
        format!("{value:.0} Mbps")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};

    fn animated_buffer(width: u16, height: u16, show_value: bool, seconds: Option<f64>) -> Buffer {
        let mut state = SpeedometerState::default();
        state.snap_to_with_peak(642.7, 780.2);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                render_animated(
                    frame,
                    frame.area(),
                    &state,
                    show_value,
                    GaugePalette {
                        background: Color::Reset,
                        accent: Color::Reset,
                        text: Color::Reset,
                        secondary: Color::Reset,
                        track: Color::Reset,
                    },
                    true,
                    seconds,
                );
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn orbital_motion_changes_geometry_without_changing_readings_or_palette() {
        let start = animated_buffer(96, 30, true, Some(0.0));
        let later = animated_buffer(96, 30, true, Some(1.7));
        assert_ne!(start, later, "even monochrome motion must be visible");
        for buffer in [&start, &later] {
            let text = buffer_text(buffer);
            assert!(text.contains("642.7 Mbps"));
            assert!(text.contains("peak 780.2"));
            assert!(text.contains("scale 1 Gbps"));
            assert!(buffer
                .content()
                .iter()
                .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset));
        }
        // Animation never paints over the exact center value, unit, or peak.
        for y in 9..17 {
            for x in 29..67 {
                assert_eq!(start[(x, y)], later[(x, y)]);
            }
        }
    }

    #[test]
    fn reduced_motion_and_invalid_clocks_produce_the_same_static_dial() {
        let still = animated_buffer(96, 30, true, None);
        assert_eq!(still, animated_buffer(96, 30, true, None));
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            assert_eq!(still, animated_buffer(96, 30, true, Some(invalid)));
        }
    }

    #[test]
    fn animated_gauge_resizes_and_keeps_unmeasured_readings_hidden() {
        for (width, height) in [(1, 1), (30, 6), (44, 12), (96, 30)] {
            for seconds in [None, Some(2.9)] {
                let buffer = animated_buffer(width, height, false, seconds);
                let text = buffer_text(&buffer);
                assert!(!text.contains("642.7"));
                assert!(!text.contains("780.2"));
                assert!(!text.contains("peak"));
                if width >= 30 {
                    assert!(text.contains("Mbps"));
                }
            }
        }
    }

    #[test]
    fn rendered_gauge_contains_value_and_unit() {
        let backend = TestBackend::new(100, 18);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut state = SpeedometerState::default();
        state.snap_to_with_peak(742.8, 742.8);

        terminal
            .draw(|frame| render(frame, frame.area(), &state, Color::Cyan, true))
            .expect("render speedometer");

        let text =
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .fold(String::new(), |mut text, cell| {
                    text.push_str(cell.symbol());
                    text
                });
        assert!(text.contains("742.8"));
        assert!(text.contains("Mbps"));
    }

    #[test]
    fn enlarged_readout_keeps_exact_values_and_hides_unmeasured_values() {
        let palette = GaugePalette {
            background: Color::Reset,
            accent: Color::Cyan,
            text: Color::Reset,
            secondary: Color::Reset,
            track: Color::Reset,
        };
        for (width, height) in [(70, 22), (100, 30)] {
            for value in [455.5, 10_000.0, 100_000.0] {
                let mut state = SpeedometerState::default();
                state.snap_to_with_peak(value, value);
                for show_value in [true, false] {
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal
                        .draw(|frame| {
                            render_center_readout(
                                frame,
                                frame.area(),
                                &state,
                                show_value,
                                palette,
                                true,
                            );
                        })
                        .unwrap();
                    let text = terminal
                        .backend()
                        .buffer()
                        .content()
                        .iter()
                        .map(|cell| cell.symbol())
                        .collect::<String>();
                    assert!(text.contains("Mbps"));
                    assert_eq!(text.contains(&format!("{value:.1}")), show_value);
                    assert_eq!(text.contains("peak"), show_value);
                }
            }
        }
    }
}
