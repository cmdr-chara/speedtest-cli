use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::{
    compare::{self, CompareResult},
    engine::{
        cloudflare::CloudflareEngine, internet::InternetEngine, librespeed::LibreSpeedEngine,
        AddressFamily, EngineConfig,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_comparison: Option<FamilyComparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FamilyComparison {
    pub ipv4: FamilyMeasurement,
    pub ipv6: FamilyMeasurement,
    pub verdict: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FamilyMeasurement {
    pub family: String,
    pub cloudflare: Option<TestResult>,
    pub librespeed: Option<TestResult>,
    pub error: Option<String>,
}

impl VerifyReport {
    pub fn pretty_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

pub async fn run(config: EngineConfig, librespeed_server: Option<&str>) -> Result<VerifyReport> {
    let (cloudflare, librespeed) = run_pair(config, librespeed_server).await?;

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
        family_comparison: None,
    })
}

async fn run_pair(
    config: EngineConfig,
    librespeed_server: Option<&str>,
) -> Result<(TestResult, TestResult)> {
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
    Ok((cloudflare, librespeed))
}

pub async fn run_family_comparison(
    config: EngineConfig,
    librespeed_server: Option<&str>,
) -> FamilyComparison {
    let ipv4 = run_family(AddressFamily::Ipv4, &config, librespeed_server).await;
    let ipv6 = run_family(AddressFamily::Ipv6, &config, librespeed_server).await;
    let verdict = match (
        &ipv4.cloudflare,
        &ipv4.librespeed,
        &ipv6.cloudflare,
        &ipv6.librespeed,
    ) {
        (Some(v4_cloudflare), Some(v4_librespeed), Some(v6_cloudflare), Some(v6_librespeed)) => {
            let v4 = (v4_cloudflare.download.mbps + v4_librespeed.download.mbps) / 2.0;
            let v6 = (v6_cloudflare.download.mbps + v6_librespeed.download.mbps) / 2.0;
            if v4.max(v6) <= f64::EPSILON {
                "Neither address family produced usable throughput data.".to_string()
            } else {
                let faster = if v4 >= v6 { "IPv4" } else { "IPv6" };
                let ratio = (v4.max(v6) - v4.min(v6)) / v4.max(v6);
                if ratio <= 0.15 {
                    "IPv4 and IPv6 were broadly comparable.".to_string()
                } else {
                    format!("{faster} was materially faster across the available backends.")
                }
            }
        }
        _ => "One address family could not complete a comparable backend pair.".to_string(),
    };
    FamilyComparison {
        ipv4,
        ipv6,
        verdict,
    }
}

async fn run_family(
    family: AddressFamily,
    config: &EngineConfig,
    librespeed_server: Option<&str>,
) -> FamilyMeasurement {
    let mut config = config.clone();
    config.family = family;
    match run_pair(config, librespeed_server).await {
        Ok((cloudflare, librespeed)) => FamilyMeasurement {
            family: family.label().to_string(),
            cloudflare: Some(cloudflare),
            librespeed: Some(librespeed),
            error: None,
        },
        Err(error) => FamilyMeasurement {
            family: family.label().to_string(),
            cloudflare: None,
            librespeed: None,
            error: Some(format!("{error:#}")),
        },
    }
}

async fn run_engine(engine: InternetEngine) -> Result<TestResult> {
    let (tx, rx) = mpsc::unbounded_channel();
    drop(rx);
    crate::runtime::deadline(std::time::Duration::from_secs(120), engine.run(tx)).await
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
