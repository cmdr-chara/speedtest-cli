pub mod cloudflare;

use std::time::Duration;

use crate::model::{TestPhase, TestResult};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub streams: usize,
    pub phase_duration: Duration,
}

#[derive(Debug, Clone)]
pub enum EngineEvent {
    PhaseChanged(TestPhase),
    IdleLatency { ping_ms: f64, jitter_ms: f64 },
    ThroughputSample { phase: TestPhase, mbps: f64 },
    LoadedLatency { phase: TestPhase, ms: f64 },
    Complete(TestResult),
    Error(String),
}
