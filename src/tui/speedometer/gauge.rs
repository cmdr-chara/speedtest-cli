use std::f64::consts::PI;

use ratatui::{
    prelude::{Alignment, Color, Frame, Line, Modifier, Rect, Span, Style},
    symbols::Marker,
    widgets::{
        canvas::{Canvas, Line as CanvasLine, Points},
        Block, Borders, Paragraph,
    },
};

use super::SpeedometerState;

const START_ANGLE: f64 = PI * 1.15;
const SWEEP_ANGLE: f64 = PI * 1.30;
const TRACK_STEPS: usize = 260;
const TRACK_RADII: &[f64] = &[1.00, 0.975, 0.95];

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &SpeedometerState,
    accent: Color,
    show_value: bool,
) {
    if area.width < 40 || area.height < 8 {
        render_fallback(frame, area, state, accent, show_value);
        return;
    }

    let maximum = state.scale_mbps().max(1.0);
    let ratio = (state.displayed_mbps() / maximum).clamp(0.0, 1.0);
    let needle_angle = angle_for_ratio(ratio);

    let track_layers: Vec<Vec<(f64, f64)>> = TRACK_RADII
        .iter()
        .map(|radius| arc_points(*radius, 1.0))
        .collect();
    let active_layers: Vec<Vec<(f64, f64)>> = TRACK_RADII
        .iter()
        .map(|radius| arc_points(*radius, ratio))
        .collect();
    let active_cap = point_on_arc(0.975, ratio);

    let labels = if area.width >= 62 && area.height >= 11 {
        scale_labels(maximum, area.width)
    } else {
        Vec::new()
    };

    let canvas = Canvas::default()
        .block(Block::default().borders(Borders::NONE))
        .marker(Marker::Braille)
        .x_bounds([-1.28, 1.28])
        .y_bounds([-0.56, 1.18])
        .paint(move |ctx| {
            for coords in &track_layers {
                ctx.draw(&Points {
                    coords,
                    color: Color::DarkGray,
                });
            }

            ctx.layer();

            if ratio > 0.0 {
                for coords in &active_layers {
                    ctx.draw(&Points {
                        coords,
                        color: accent,
                    });
                }
                ctx.draw(&Points {
                    coords: &[active_cap],
                    color: Color::White,
                });
            }

            draw_ticks(ctx, ratio, accent);
            draw_needle(ctx, needle_angle, accent);

            for (x, y, label) in &labels {
                ctx.print(
                    *x,
                    *y,
                    Line::from(Span::styled(
                        label.clone(),
                        Style::default().fg(Color::Gray),
                    )),
                );
            }
        });

    frame.render_widget(canvas, area);
    render_center_readout(frame, area, state, show_value);
}

fn draw_ticks(ctx: &mut ratatui::widgets::canvas::Context<'_>, ratio: f64, accent: Color) {
    for index in 0..=20 {
        let fraction = index as f64 / 20.0;
        let angle = angle_for_ratio(fraction);
        let major = index % 5 == 0;
        let inner_radius = if major { 0.80 } else { 0.865 };
        let outer_radius = if major { 1.065 } else { 1.025 };
        let color = if fraction <= ratio {
            if major { accent } else { Color::Gray }
        } else if major {
            Color::Gray
        } else {
            Color::DarkGray
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

fn draw_needle(
    ctx: &mut ratatui::widgets::canvas::Context<'_>,
    needle_angle: f64,
    accent: Color,
) {
    ctx.draw(&CanvasLine {
        x1: 0.0,
        y1: 0.0,
        x2: 0.82 * needle_angle.cos(),
        y2: 0.82 * needle_angle.sin(),
        color: Color::DarkGray,
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
        color: Color::Gray,
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
        color: Color::White,
    });
}

fn render_center_readout(
    frame: &mut Frame,
    area: Rect,
    state: &SpeedometerState,
    show_value: bool,
) {
    if area.width < 20 || area.height < 6 {
        return;
    }

    let width = area.width.min(38);
    let x = area.x + (area.width - width) / 2;
    let y = area.y + area.height.saturating_mul(53) / 100;
    let height = area.height.saturating_sub(y.saturating_sub(area.y)).min(3);
    if height == 0 {
        return;
    }

    let value = if show_value {
        format!("{:.1}", state.displayed_mbps())
    } else {
        "—".to_string()
    };

    let mut lines = vec![
        Line::from(Span::styled(
            value,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled("Mbps", Style::default().fg(Color::Gray))),
    ];

    if height >= 3 && show_value {
        lines.push(Line::from(Span::styled(
            format!(
                "peak {:.1}  •  scale {}",
                state.peak_mbps(),
                format_scale(state.scale_mbps())
            ),
            Style::default().fg(Color::DarkGray),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center),
        Rect::new(x, y, width, height),
    );
}

fn render_fallback(
    frame: &mut Frame,
    area: Rect,
    state: &SpeedometerState,
    accent: Color,
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
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center),
        area,
    );
}

fn arc_points(radius: f64, fraction: f64) -> Vec<(f64, f64)> {
    let fraction = fraction.clamp(0.0, 1.0);
    let steps = ((TRACK_STEPS as f64 * fraction).ceil() as usize).max(1);
    (0..=steps)
        .map(|step| {
            let t = fraction * step as f64 / steps as f64;
            let angle = angle_for_ratio(t);
            (radius * angle.cos(), radius * angle.sin())
        })
        .collect()
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
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn rendered_gauge_contains_value_and_unit() {
        let backend = TestBackend::new(100, 18);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut state = SpeedometerState::default();
        state.snap_to_with_peak(742.8, 742.8);

        terminal
            .draw(|frame| render(frame, frame.area(), &state, Color::Cyan, true))
            .expect("render speedometer");

        let text = terminal
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
}
