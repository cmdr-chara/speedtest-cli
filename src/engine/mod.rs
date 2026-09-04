#[path = "cloudflare_adaptive.rs"]
pub mod cloudflare;
pub(crate) mod http;
pub mod internet;
pub mod librespeed;

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

impl EngineConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=16).contains(&self.streams),
            "streams must be between 1 and 16"
        );
        anyhow::ensure!(
            !self.phase_duration.is_zero() && self.phase_duration <= Duration::from_secs(30),
            "phase duration must be positive and at most 30 seconds"
        );
        Ok(())
    }
}

/// Keep latency probes, sampling, and workers in one cancellation scope.
pub(crate) async fn finish_phase(
    mut workers: tokio::task::JoinSet<anyhow::Result<()>>,
    samples: impl std::future::Future<Output = ()>,
    loaded: impl std::future::Future<Output = Vec<f64>>,
) -> anyhow::Result<Vec<f64>> {
    use anyhow::Context;
    let collect = async {
        while let Some(result) = workers.join_next().await {
            result.context("transfer worker failed")??;
        }
        Ok::<_, anyhow::Error>(())
    };
    let (_, _, latency) = tokio::try_join!(
        collect,
        async {
            samples.await;
            Ok(())
        },
        async { Ok(loaded.await) },
    )?;
    Ok(latency)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unbounded_library_configuration() {
        for streams in [0, 17, usize::MAX] {
            assert!(EngineConfig {
                streams,
                phase_duration: Duration::from_secs(1)
            }
            .validate()
            .is_err());
        }
        assert!(EngineConfig {
            streams: 1,
            phase_duration: Duration::ZERO
        }
        .validate()
        .is_err());
    }

    #[tokio::test]
    async fn worker_failure_does_not_wait_for_sampler_or_loaded_probe() {
        let mut workers = tokio::task::JoinSet::new();
        workers.spawn(async { anyhow::bail!("fixture failure") });
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            finish_phase(workers, std::future::pending(), std::future::pending()),
        )
        .await;
        assert!(result.unwrap().is_err());
    }
}

#[cfg(test)]
mod test_support;
