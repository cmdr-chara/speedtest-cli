mod gauge;

use std::time::Duration;

pub use gauge::render;

const SPRING_STIFFNESS: f64 = 42.0;
const SPRING_DAMPING: f64 = 12.0;
const MAX_ANIMATION_STEP: f64 = 0.100;

#[derive(Debug, Clone)]
pub struct SpeedometerState {
    target_mbps: f64,
    displayed_mbps: f64,
    velocity: f64,
    scale_mbps: f64,
    peak_mbps: f64,
}

impl Default for SpeedometerState {
    fn default() -> Self {
        Self {
            target_mbps: 0.0,
            displayed_mbps: 0.0,
            velocity: 0.0,
            scale_mbps: 100.0,
            peak_mbps: 0.0,
        }
    }
}

impl SpeedometerState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn set_target(&mut self, value: f64) {
        let value = value.max(0.0);
        self.target_mbps = value;
        self.peak_mbps = self.peak_mbps.max(value);

        let desired_scale = scale_for(self.peak_mbps);
        if desired_scale > self.scale_mbps {
            self.scale_mbps = desired_scale;
        }
    }

    pub fn snap_to_with_peak(&mut self, value: f64, peak: f64) {
        let value = value.max(0.0);
        let peak = peak.max(value);
        self.target_mbps = value;
        self.displayed_mbps = value;
        self.velocity = 0.0;
        self.peak_mbps = peak;
        self.scale_mbps = scale_for(peak);
    }

    pub fn tick(&mut self, delta: Duration) {
        let dt = delta.as_secs_f64().clamp(0.0, MAX_ANIMATION_STEP);
        if dt <= f64::EPSILON {
            return;
        }

        let error = self.target_mbps - self.displayed_mbps;
        let acceleration = error * SPRING_STIFFNESS - self.velocity * SPRING_DAMPING;
        self.velocity += acceleration * dt;
        self.displayed_mbps += self.velocity * dt;

        if self.displayed_mbps < 0.0 {
            self.displayed_mbps = 0.0;
            self.velocity = 0.0;
        }

        if error.abs() < 0.05 && self.velocity.abs() < 0.05 {
            self.displayed_mbps = self.target_mbps;
            self.velocity = 0.0;
        }
    }

    pub const fn displayed_mbps(&self) -> f64 {
        self.displayed_mbps
    }

    pub const fn peak_mbps(&self) -> f64 {
        self.peak_mbps
    }

    pub const fn scale_mbps(&self) -> f64 {
        self.scale_mbps
    }
}

fn scale_for(value: f64) -> f64 {
    const SCALES: &[f64] = &[100.0, 250.0, 500.0, 1_000.0, 2_500.0, 5_000.0, 10_000.0];
    let value = value.max(0.0);
    SCALES
        .iter()
        .copied()
        .find(|candidate| value <= *candidate)
        .unwrap_or_else(|| ((value / 5_000.0).ceil() * 5_000.0).max(10_000.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_only_grows_until_reset() {
        let mut state = SpeedometerState::default();
        state.set_target(840.0);
        assert_eq!(state.scale_mbps(), 1_000.0);

        state.set_target(120.0);
        assert_eq!(state.scale_mbps(), 1_000.0);

        state.reset();
        state.set_target(120.0);
        assert_eq!(state.scale_mbps(), 250.0);
    }

    #[test]
    fn spring_animation_converges_on_target() {
        let mut state = SpeedometerState::default();
        state.set_target(800.0);

        for _ in 0..180 {
            state.tick(Duration::from_millis(16));
        }

        assert!((state.displayed_mbps() - 800.0).abs() < 0.1);
    }

    #[test]
    fn snap_resets_scale_and_animation() {
        let mut state = SpeedometerState::default();
        state.set_target(2_000.0);
        state.tick(Duration::from_millis(100));
        state.snap_to_with_peak(420.0, 420.0);

        assert_eq!(state.displayed_mbps(), 420.0);
        assert_eq!(state.peak_mbps(), 420.0);
        assert_eq!(state.scale_mbps(), 500.0);
    }
}
