use std::f64::consts::PI;

use ratatui::{
    prelude::{Color, Frame, Rect},
    symbols::Marker,
    widgets::{
        canvas::{Canvas, Line as CanvasLine, Points},
        Block, Borders,
    },
};

pub fn render(frame: &mut Frame, area: Rect, value: f64, maximum: f64, accent: Color) {
    let maximum = maximum.max(1.0);
    let ratio = (value / maximum).clamp(0.0, 1.0);
    let needle_angle = PI - (PI * ratio);
    let needle_x = 0.78 * needle_angle.cos();
    let needle_y = 0.78 * needle_angle.sin();

    let arc: Vec<(f64, f64)> = (0..=160)
        .map(|step| {
            let angle = PI - (PI * step as f64 / 160.0);
            (angle.cos(), angle.sin())
        })
        .collect();

    let ticks: Vec<(f64, f64)> = (0..=10)
        .map(|step| {
            let angle = PI - (PI * step as f64 / 10.0);
            (0.92 * angle.cos(), 0.92 * angle.sin())
        })
        .collect();

    let canvas = Canvas::default()
        .block(Block::default().borders(Borders::NONE))
        .marker(Marker::Braille)
        .x_bounds([-1.15, 1.15])
        .y_bounds([-0.14, 1.10])
        .paint(move |ctx| {
            ctx.draw(&Points {
                coords: &arc,
                color: Color::DarkGray,
            });
            ctx.draw(&Points {
                coords: &ticks,
                color: Color::Gray,
            });
            ctx.draw(&CanvasLine {
                x1: 0.0,
                y1: 0.0,
                x2: needle_x,
                y2: needle_y,
                color: accent,
            });
            ctx.draw(&Points {
                coords: &[(0.0, 0.0)],
                color: accent,
            });
        });

    frame.render_widget(canvas, area);
}

pub fn scale_for(value: f64) -> f64 {
    const SCALES: &[f64] = &[100.0, 250.0, 500.0, 1_000.0, 2_500.0, 5_000.0, 10_000.0];
    SCALES
        .iter()
        .copied()
        .find(|candidate| value <= *candidate)
        .unwrap_or_else(|| ((value / 5_000.0).ceil() * 5_000.0).max(10_000.0))
}
