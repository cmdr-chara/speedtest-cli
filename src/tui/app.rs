use std::{collections::VecDeque, time::Duration};

use crate::{
    engine::EngineEvent,
    model::{TestPhase, TestResult},
};

use super::speedometer::SpeedometerState;

#[derive(Debug)]
pub(super) struct App {
    pub(super) phase: TestPhase,
    pub(super) speedometer: SpeedometerState,
    pub(super) download_mbps: Option<f64>,
    pub(super) upload_mbps: Option<f64>,
    pub(super) ping_ms: Option<f64>,
    pub(super) jitter_ms: Option<f64>,
    pub(super) download_loaded_ms: Option<f64>,
    pub(super) upload_loaded_ms: Option<f64>,
    pub(super) samples: VecDeque<f64>,
    download_peak_mbps: f64,
    pub(super) result: Option<TestResult>,
    pub(super) error: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            phase: TestPhase::Preparing,
            speedometer: SpeedometerState::default(),
            download_mbps: None,
            upload_mbps: None,
            ping_ms: None,
            jitter_ms: None,
            download_loaded_ms: None,
            upload_loaded_ms: None,
            samples: VecDeque::with_capacity(90),
            download_peak_mbps: 0.0,
            result: None,
            error: None,
        }
    }
}

impl App {
    pub(super) fn apply(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::PhaseChanged(phase) => {
                if matches!(phase, TestPhase::Download | TestPhase::Upload) {
                    self.speedometer.reset();
                    self.samples.clear();
                    if phase == TestPhase::Download {
                        self.download_peak_mbps = 0.0;
                    }
                }
                self.phase = phase;
            }
            EngineEvent::IdleLatency { ping_ms, jitter_ms } => {
                self.ping_ms = Some(ping_ms);
                self.jitter_ms = Some(jitter_ms);
            }
            EngineEvent::ThroughputSample { phase, mbps } => {
                self.phase = phase;
                self.speedometer.set_target(mbps);
                match phase {
                    TestPhase::Download => {
                        self.download_mbps = Some(mbps);
                        self.download_peak_mbps = self.download_peak_mbps.max(mbps);
                    }
                    TestPhase::Upload => self.upload_mbps = Some(mbps),
                    _ => {}
                }

                if self.samples.len() == 90 {
                    self.samples.pop_front();
                }
                self.samples.push_back(mbps);
            }
            EngineEvent::LoadedLatency { phase, ms } => match phase {
                TestPhase::Download => self.download_loaded_ms = Some(ms),
                TestPhase::Upload => self.upload_loaded_ms = Some(ms),
                _ => {}
            },
            EngineEvent::Complete(result) => {
                self.download_mbps = Some(result.download.mbps);
                self.upload_mbps = Some(result.upload.mbps);
                self.download_loaded_ms = result.latency.download_loaded_ms;
                self.upload_loaded_ms = result.latency.upload_loaded_ms;
                self.speedometer.snap_to_with_peak(
                    result.download.mbps,
                    self.download_peak_mbps.max(result.download.mbps),
                );
                self.phase = TestPhase::Complete;
                self.result = Some(result);
            }
            EngineEvent::Error(error) => self.error = Some(error),
        }
    }

    pub(super) fn tick(&mut self, delta: Duration) -> bool {
        self.speedometer.tick(delta)
    }

    pub(super) fn is_complete(&self) -> bool {
        self.result.is_some()
    }

    pub(super) fn footer_source(&self) -> String {
        self.result.as_ref().map_or_else(
            || "Internet speed test".to_string(),
            |result| format!("{} ({})", result.server.host, result.backend),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_keeps_result_and_focuses_download() {
        let mut app = App {
            download_peak_mbps: 812.0,
            ..Default::default()
        };
        app.apply(EngineEvent::Complete(TestResult {
            timestamp: chrono::Utc::now(),
            backend: "cloudflare".to_string(),
            server: crate::model::ServerInfo {
                host: "speed.cloudflare.com".to_string(),
                name: "Cloudflare Edge".to_string(),
            },
            latency: crate::model::LatencyResult {
                idle_ms: 8.0,
                jitter_ms: 1.0,
                download_loaded_ms: Some(15.0),
                upload_loaded_ms: Some(18.0),
                packet_loss_percent: None,
            },
            download: crate::model::ThroughputResult {
                mbps: 780.0,
                bytes: 1,
                seconds: 1.0,
            },
            upload: crate::model::ThroughputResult {
                mbps: 210.0,
                bytes: 1,
                seconds: 1.0,
            },
            analysis: None,
        }));

        assert!(app.is_complete());
        assert_eq!(app.phase, TestPhase::Complete);
        assert_eq!(app.speedometer.displayed_mbps(), 780.0);
        assert_eq!(app.speedometer.peak_mbps(), 812.0);
        assert_eq!(app.footer_source(), "speed.cloudflare.com (cloudflare)");
    }

    #[test]
    fn footer_is_backend_neutral_until_a_result_exists() {
        assert_eq!(App::default().footer_source(), "Internet speed test");
    }
}
