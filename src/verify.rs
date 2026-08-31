use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::{
    compare::{self, CompareResult},
    engine::{
        cloudflare::CloudflareEngine, internet::InternetEngine, librespeed::LibreSpeedEngine,
        EngineConfig, EngineEvent,
    },
    loss::{self, PacketLossResult},
    model::TestResult,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyReport {
    pub timestamp: DateTime<Utc>,
    pub cloudflare: TestResult,
    pub librespeed: TestResult,
    pub comparison: CompareResult,
    pub icmp_loss: Option<PacketLossResult>,
    pub consistent: bool,
    pub verdict: String,
}

impl VerifyReport {
    pub fn pretty_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

pub async fn run(config: EngineConfig, librespeed_server: Option<&str>) -> Result<VerifyReport> {
    let cloudflare = InternetEngine::Cloudflare(CloudflareEngine::new(config.clone())?);
    // Construct both engines before starting either measurement so malformed custom
    // LibreSpeed settings fail immediately instead of after a full Cloudflare run.
    let librespeed = InternetEngine::LibreSpeed(LibreSpeedEngine::new(config, librespeed_server)?);

    let cloudflare = run_engine(cloudflare)
        .await
        .context("Cloudflare verification run failed")?;

    let librespeed = run_engine(librespeed)
        .await
        .context("LibreSpeed verification run failed")?;

    let comparison = compare::compare(&cloudflare, &librespeed);
    let consistent = throughput_agrees(cloudflare.download.mbps, librespeed.download.mbps, 0.30)
        && throughput_agrees(cloudflare.upload.mbps, librespeed.upload.mbps, 0.35)
        && latency_agrees(cloudflare.latency.idle_ms, librespeed.latency.idle_ms);
    let verdict = if consistent {
        "Backends broadly agree; the measured capacity is reproducible across independent test infrastructure."
    } else {
        "Backends disagree materially; routing, server capacity, peering, or backend methodology may be influencing the result."
    }
    .to_string();

    let icmp_loss = loss::measure(loss::default_target(), 20).await.ok();

    Ok(VerifyReport {
        timestamp: Utc::now(),
        cloudflare,
        librespeed,
        comparison,
        icmp_loss,
        consistent,
        verdict,
    })
}

async fn run_engine(engine: InternetEngine) -> Result<TestResult> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let runner = engine.clone();
    let handle = tokio::spawn(async move { runner.run(tx).await });

    while let Some(event) = rx.recv().await {
        match event {
            EngineEvent::Complete(result) => {
                handle
                    .await
                    .context("verification engine task panicked")??;
                return Ok(result);
            }
            EngineEvent::Error(error) => anyhow::bail!(error),
            _ => {}
        }
    }
    handle.await.context("verification engine task panicked")?
}

fn throughput_agrees(left: f64, right: f64, tolerance: f64) -> bool {
    let high = left.max(right);
    let low = left.min(right);
    high <= f64::EPSILON || (high - low) / high <= tolerance
}

fn latency_agrees(left: f64, right: f64) -> bool {
    let delta = (left - right).abs();
    delta <= 10.0 || delta / left.max(right).max(1.0) <= 0.50
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_agreement_tolerates_normal_variation() {
        assert!(throughput_agrees(900.0, 760.0, 0.30));
        assert!(!throughput_agrees(900.0, 500.0, 0.30));
        assert!(latency_agrees(10.0, 17.0));
        assert!(!latency_agrees(10.0, 35.0));
    }
}
