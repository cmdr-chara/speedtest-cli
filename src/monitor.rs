//! JSONL records for explicit, repeatable connection monitoring.

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{analysis, model::TestResult};

pub const SCHEMA_VERSION: u8 = 1;
pub const REPORT_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorRecord {
    pub schema_version: u8,
    pub sequence: u32,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<TestResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl MonitorRecord {
    pub fn success(
        sequence: u32,
        started_at: DateTime<Utc>,
        completed_at: DateTime<Utc>,
        result: TestResult,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            sequence,
            started_at,
            completed_at,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(
        sequence: u32,
        started_at: DateTime<Utc>,
        completed_at: DateTime<Utc>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            sequence,
            started_at,
            completed_at,
            ok: false,
            result: None,
            error: Some(error.into()),
        }
    }

    /// Validate the invariants that make a JSONL record meaningful. Keeping
    /// this at the reader boundary prevents malformed hand-written records
    /// from being silently counted in an offline report.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == SCHEMA_VERSION,
            "unsupported monitor schema version {}",
            self.schema_version
        );
        anyhow::ensure!(
            self.completed_at >= self.started_at,
            "monitor record completed before it started"
        );
        if self.ok {
            self.result
                .as_ref()
                .context("successful monitor record has no result")?
                .validate()
                .context("successful monitor record contains an invalid result")?;
            anyhow::ensure!(
                self.error.is_none(),
                "successful monitor record contains an error"
            );
        } else {
            anyhow::ensure!(
                self.result.is_none(),
                "failed monitor record contains a result"
            );
            anyhow::ensure!(
                self.error
                    .as_ref()
                    .is_some_and(|error| !error.trim().is_empty()),
                "failed monitor record has no error"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MonitorMetric {
    pub samples: usize,
    pub min: f64,
    pub median: f64,
    pub p95: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonitorReport {
    pub schema_version: u8,
    pub generated_at: DateTime<Utc>,
    pub attempts: usize,
    pub successes: usize,
    pub failures: usize,
    pub success_rate_percent: f64,
    pub first_started_at: Option<DateTime<Utc>>,
    pub last_completed_at: Option<DateTime<Utc>>,
    pub duration_seconds: f64,
    pub max_failure_streak: usize,
    pub current_failure_streak: usize,
    pub download_mbps: Option<MonitorMetric>,
    pub upload_mbps: Option<MonitorMetric>,
    pub idle_latency_ms: Option<MonitorMetric>,
    pub jitter_ms: Option<MonitorMetric>,
    pub quality_score: Option<MonitorMetric>,
    pub recent_failures: Vec<String>,
}

impl MonitorReport {
    pub fn empty() -> Self {
        Self {
            schema_version: REPORT_SCHEMA_VERSION,
            generated_at: Utc::now(),
            attempts: 0,
            successes: 0,
            failures: 0,
            success_rate_percent: 0.0,
            first_started_at: None,
            last_completed_at: None,
            duration_seconds: 0.0,
            max_failure_streak: 0,
            current_failure_streak: 0,
            download_mbps: None,
            upload_mbps: None,
            idle_latency_ms: None,
            jitter_ms: None,
            quality_score: None,
            recent_failures: Vec::new(),
        }
    }
}

pub fn summarize(records: &[MonitorRecord]) -> MonitorReport {
    let mut report = MonitorReport::empty();
    report.attempts = records.len();
    if records.is_empty() {
        return report;
    }

    report.successes = records.iter().filter(|record| record.ok).count();
    report.failures = report.attempts - report.successes;
    report.success_rate_percent = report.successes as f64 / report.attempts as f64 * 100.0;
    report.first_started_at = records.iter().map(|record| record.started_at).min();
    report.last_completed_at = records.iter().map(|record| record.completed_at).max();
    report.duration_seconds = report
        .first_started_at
        .zip(report.last_completed_at)
        .map_or(0.0, |(first, last)| {
            (last - first).num_milliseconds().max(0) as f64 / 1000.0
        });

    let mut current = 0;
    let mut max_streak = 0;
    let mut downloads = Vec::new();
    let mut uploads = Vec::new();
    let mut latencies = Vec::new();
    let mut jitters = Vec::new();
    let mut qualities = Vec::new();
    let mut failures = Vec::new();
    for record in records {
        if let Some(result) = record.result.as_ref() {
            downloads.push(result.download.mbps);
            uploads.push(result.upload.mbps);
            latencies.push(result.latency.idle_ms);
            jitters.push(result.latency.jitter_ms);
            if let Some(quality) = result
                .analysis
                .as_ref()
                .map(|analysis| analysis.quality.score)
            {
                qualities.push(f64::from(quality));
            }
            current = 0;
        } else {
            current += 1;
            max_streak = max_streak.max(current);
            if failures.len() < 5 {
                if let Some(error) = record.error.as_deref() {
                    failures.push(error.to_string());
                }
            }
        }
    }
    report.current_failure_streak = current;
    report.max_failure_streak = max_streak;
    report.recent_failures = failures;
    report.download_mbps = metric(&downloads);
    report.upload_mbps = metric(&uploads);
    report.idle_latency_ms = metric(&latencies);
    report.jitter_ms = metric(&jitters);
    report.quality_score = metric(&qualities);
    report
}

fn metric(values: &[f64]) -> Option<MonitorMetric> {
    let distribution = analysis::distribution(values)?;
    Some(MonitorMetric {
        samples: distribution.samples,
        min: distribution.min_ms,
        median: distribution.median_ms,
        p95: distribution.p95_ms,
        max: distribution.max_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_record_is_explicit_and_versioned() {
        let record = MonitorRecord::failure(2, Utc::now(), Utc::now(), "fixture failed");
        assert_eq!(record.schema_version, SCHEMA_VERSION);
        assert!(!record.ok);
        assert_eq!(record.error.as_deref(), Some("fixture failed"));
        assert!(record.result.is_none());
        record.validate().unwrap();
    }

    #[test]
    fn report_counts_failures_and_uses_only_successful_samples_for_metrics() {
        let now = Utc::now();
        let result: TestResult =
            serde_json::from_str(include_str!("../tests/fixtures/result.json")).unwrap();
        let first = MonitorRecord::success(1, now, now, result.clone());
        let failure = MonitorRecord::failure(2, now, now, "timeout");
        let mut faster = result;
        faster.download.mbps = 200.0;
        let last = MonitorRecord::success(3, now, now, faster);
        let report = summarize(&[first, failure, last]);
        assert_eq!(
            (report.attempts, report.successes, report.failures),
            (3, 2, 1)
        );
        assert!((report.success_rate_percent - 66.666666).abs() < 0.001);
        assert_eq!(report.download_mbps.as_ref().unwrap().samples, 2);
        assert_eq!(report.max_failure_streak, 1);
        assert_eq!(report.current_failure_streak, 0);
        assert_eq!(report.recent_failures, vec!["timeout"]);
    }
}
