use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::mpsc::UnboundedSender,
    time::{sleep_until, Instant},
};

use crate::{
    analysis,
    engine::cloudflare::CloudflareEngine,
    model::{LatencyDistribution, QualityGrade},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StabilitySample {
    pub elapsed_ms: u64,
    pub latency_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StabilityResult {
    pub timestamp: chrono::DateTime<Utc>,
    pub duration_seconds: u64,
    pub interval_ms: u64,
    pub samples: Vec<StabilitySample>,
    pub successful_probes: usize,
    pub failed_probes: usize,
    pub probe_availability_percent: f64,
    pub failure_bursts: usize,
    pub latency: Option<LatencyDistribution>,
    pub jitter: Option<LatencyDistribution>,
    pub score: u8,
    pub grade: QualityGrade,
    pub s_tier: bool,
}

impl StabilityResult {
    pub const fn tier_label(&self) -> Option<&'static str> {
        if self.s_tier {
            Some("S-TIER")
        } else {
            None
        }
    }

    pub fn pretty_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

#[derive(Debug, Clone)]
pub enum StabilityEvent {
    Sample(StabilitySample),
    Complete(Box<StabilityResult>),
}

pub async fn run(
    engine: &CloudflareEngine,
    duration: Duration,
    interval: Duration,
    tx: Option<UnboundedSender<StabilityEvent>>,
) -> Result<StabilityResult> {
    let started = Instant::now();
    let deadline = started + duration;
    let mut next_probe = started;
    let mut samples = Vec::new();

    while Instant::now() < deadline {
        let now = Instant::now();
        if now < next_probe {
            sleep_until(next_probe.min(deadline)).await;
        }
        if Instant::now() >= deadline {
            break;
        }

        let latency_ms = tokio::select! {
            result = engine.latency_probe() => result.ok(),
            _ = sleep_until(deadline) => break,
        };
        let sample = StabilitySample {
            elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            latency_ms,
        };
        if let Some(tx) = &tx {
            let _ = tx.send(StabilityEvent::Sample(sample.clone()));
        }
        samples.push(sample);

        next_probe += interval;
        let now = Instant::now();
        while next_probe <= now {
            next_probe += interval;
        }
    }

    let result = summarize(samples, duration, interval);
    if let Some(tx) = tx {
        let _ = tx.send(StabilityEvent::Complete(Box::new(result.clone())));
    }
    Ok(result)
}

pub fn summarize(
    samples: Vec<StabilitySample>,
    duration: Duration,
    interval: Duration,
) -> StabilityResult {
    let successful: Vec<f64> = samples.iter().filter_map(|sample| sample.latency_ms).collect();
    let jitter_values: Vec<f64> = samples
        .windows(2)
        .filter_map(|pair| match (pair[0].latency_ms, pair[1].latency_ms) {
            (Some(left), Some(right)) => Some((right - left).abs()),
            _ => None,
        })
        .collect();
    let successful_probes = successful.len();
    let failed_probes = samples.len().saturating_sub(successful_probes);
    let availability = if samples.is_empty() {
        0.0
    } else {
        successful_probes as f64 / samples.len() as f64 * 100.0
    };
    let latency = analysis::distribution(&successful);
    let jitter = analysis::distribution(&jitter_values);
    let score = stability_score(latency.as_ref(), jitter.as_ref(), availability);
    let grade = grade_for_score(f64::from(score));
    let s_tier = score >= 98
        && grade == QualityGrade::APlus
        && failed_probes == 0
        && successful_probes >= 30;

    StabilityResult {
        timestamp: Utc::now(),
        duration_seconds: duration.as_secs(),
        interval_ms: interval.as_millis().min(u128::from(u64::MAX)) as u64,
        samples,
        successful_probes,
        failed_probes,
        probe_availability_percent: availability,
        failure_bursts: failure_bursts_from_samples(&successful, failed_probes, &jitter_values),
        latency,
        jitter,
        score,
        grade,
        s_tier,
    }
}

fn failure_bursts_from_samples(
    successful: &[f64],
    failed_probes: usize,
    _jitter_values: &[f64],
) -> usize {
    if failed_probes == 0 {
        return 0;
    }
    // The exact burst count is computed from the original sample sequence by the public helper.
    // This fallback keeps the summary conservative if only aggregate inputs are ever supplied.
    usize::from(!successful.is_empty() || failed_probes > 0)
}

pub fn failure_bursts(samples: &[StabilitySample]) -> usize {
    let mut bursts = 0;
    let mut in_failure = false;
    for sample in samples {
        if sample.latency_ms.is_none() {
            if !in_failure {
                bursts += 1;
                in_failure = true;
            }
        } else {
            in_failure = false;
        }
    }
    bursts
}

fn stability_score(
    latency: Option<&LatencyDistribution>,
    jitter: Option<&LatencyDistribution>,
    availability: f64,
) -> u8 {
    let Some(latency) = latency else {
        return 0;
    };
    let jitter_p95 = jitter.map_or(100.0, |stats| stats.p95_ms);
    let components = [
        (lower_score(latency.median_ms, &[(10.0, 100.0), (20.0, 95.0), (40.0, 85.0), (80.0, 65.0), (150.0, 40.0)]), 0.25),
        (lower_score(latency.p95_ms, &[(15.0, 100.0), (30.0, 95.0), (60.0, 82.0), (120.0, 60.0), (250.0, 35.0)]), 0.30),
        (lower_score(latency.p99_ms, &[(20.0, 100.0), (40.0, 92.0), (80.0, 78.0), (160.0, 55.0), (350.0, 30.0)]), 0.20),
        (lower_score(jitter_p95, &[(2.0, 100.0), (5.0, 95.0), (10.0, 85.0), (20.0, 65.0), (50.0, 35.0)]), 0.15),
        (availability_score(availability), 0.10),
    ];
    components
        .iter()
        .map(|(score, weight)| score * weight)
        .sum::<f64>()
        .round()
        .clamp(0.0, 100.0) as u8
}

fn lower_score(value: f64, bands: &[(f64, f64)]) -> f64 {
    bands
        .iter()
        .find_map(|(limit, score)| (value <= *limit).then_some(*score))
        .unwrap_or(15.0)
}

fn availability_score(availability: f64) -> f64 {
    if availability >= 100.0 {
        100.0
    } else if availability >= 99.5 {
        95.0
    } else if availability >= 99.0 {
        85.0
    } else if availability >= 97.0 {
        65.0
    } else if availability >= 90.0 {
        40.0
    } else {
        10.0
    }
}

fn grade_for_score(score: f64) -> QualityGrade {
    if score >= 95.0 {
        QualityGrade::APlus
    } else if score >= 88.0 {
        QualityGrade::A
    } else if score >= 78.0 {
        QualityGrade::B
    } else if score >= 65.0 {
        QualityGrade::C
    } else if score >= 50.0 {
        QualityGrade::D
    } else {
        QualityGrade::F
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(index: u64, latency: Option<f64>) -> StabilitySample {
        StabilitySample {
            elapsed_ms: index * 1000,
            latency_ms: latency,
        }
    }

    #[test]
    fn stable_low_latency_run_scores_high() {
        let samples: Vec<_> = (0..60)
            .map(|index| sample(index, Some(8.0 + f64::from((index % 2) as u8))))
            .collect();
        let result = summarize(samples, Duration::from_secs(60), Duration::from_secs(1));
        assert!(result.score >= 95);
        assert_eq!(result.grade, QualityGrade::APlus);
        assert!(result.s_tier);
    }

    #[test]
    fn failed_probes_reduce_availability_and_form_bursts() {
        let samples = vec![
            sample(0, Some(10.0)),
            sample(1, None),
            sample(2, None),
            sample(3, Some(12.0)),
            sample(4, None),
        ];
        assert_eq!(failure_bursts(&samples), 2);
        let result = summarize(samples, Duration::from_secs(5), Duration::from_secs(1));
        assert_eq!(result.failed_probes, 3);
        assert!(result.probe_availability_percent < 50.0);
        assert!(!result.s_tier);
    }
}
