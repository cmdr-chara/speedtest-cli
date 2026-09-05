//! Offline, explicit service-level checks over a canonical result.
use std::io::Read;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::model::TestResult;

pub const MAX_RESULT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Default, Clone)]
pub struct Thresholds {
    pub min_download: Option<f64>,
    pub min_upload: Option<f64>,
    pub max_latency: Option<f64>,
    pub max_jitter: Option<f64>,
    pub max_loaded_latency: Option<f64>,
    pub max_age: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub schema_version: u8,
    pub passed: bool,
    pub result_timestamp: DateTime<Utc>,
    pub checks: Vec<MetricCheck>,
}

#[derive(Debug, Serialize)]
pub struct MetricCheck {
    pub metric: &'static str,
    pub unit: &'static str,
    pub actual: Option<f64>,
    pub operator: &'static str,
    pub limit: f64,
    pub passed: bool,
}

pub fn read_result(reader: impl Read) -> Result<TestResult> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_RESULT_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("failed to read result")?;
    if bytes.len() as u64 > MAX_RESULT_BYTES {
        bail!("result exceeds the 4 MiB input limit");
    }
    serde_json::from_slice(&bytes).context("expected one canonical speedtest JSON result")
}

pub fn evaluate(
    result: &TestResult,
    limits: &Thresholds,
    now: DateTime<Utc>,
) -> Result<CheckReport> {
    let mut checks = Vec::new();
    let candidates = [
        (
            "download",
            "Mbps",
            Some(result.download.mbps),
            limits.min_download,
            true,
        ),
        (
            "upload",
            "Mbps",
            Some(result.upload.mbps),
            limits.min_upload,
            true,
        ),
        (
            "idle_latency",
            "ms",
            Some(result.latency.idle_ms),
            limits.max_latency,
            false,
        ),
        (
            "jitter",
            "ms",
            Some(result.latency.jitter_ms),
            limits.max_jitter,
            false,
        ),
        (
            "download_loaded_latency",
            "ms",
            result.latency.download_loaded_ms,
            limits.max_loaded_latency,
            false,
        ),
        (
            "upload_loaded_latency",
            "ms",
            result.latency.upload_loaded_ms,
            limits.max_loaded_latency,
            false,
        ),
    ];
    for (metric, unit, actual, limit, minimum) in candidates {
        if let Some(limit) = limit {
            if !limit.is_finite() || limit < 0.0 {
                bail!("thresholds must be finite, non-negative numbers");
            }
            if actual.is_some_and(|v| !v.is_finite() || v < 0.0) {
                bail!("result contains an invalid {metric} value");
            }
            checks.push(MetricCheck {
                metric,
                unit,
                actual,
                operator: if minimum { ">=" } else { "<=" },
                limit,
                passed: actual.is_some_and(|v| if minimum { v >= limit } else { v <= limit }),
            });
        }
    }
    if let Some(max_age) = limits.max_age {
        let age = now
            .signed_duration_since(result.timestamp)
            .num_milliseconds() as f64
            / 1000.0;
        checks.push(MetricCheck {
            metric: "age",
            unit: "s",
            actual: Some(age),
            operator: "<=",
            limit: max_age as f64,
            // A future timestamp must not bypass freshness checks.
            passed: age >= 0.0 && age <= max_age as f64,
        });
    }
    if checks.is_empty() {
        bail!("provide at least one threshold; see `speedtest check --help`");
    }
    Ok(CheckReport {
        schema_version: 1,
        passed: checks.iter().all(|check| check.passed),
        result_timestamp: result.timestamp,
        checks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> TestResult {
        serde_json::from_value(serde_json::json!({
            "timestamp":"2026-01-01T00:00:00Z", "backend":"fixture",
            "server":{"host":"localhost","name":"fixture"},
            "latency":{"idle_ms":10.0,"jitter_ms":2.0,"download_loaded_ms":null,"upload_loaded_ms":30.0,"packet_loss_percent":null},
            "download":{"mbps":100.0,"bytes":12500000,"seconds":1.0},
            "upload":{"mbps":20.0,"bytes":2500000,"seconds":1.0}
        })).unwrap()
    }

    #[test]
    fn exact_thresholds_pass_and_missing_metrics_fail_closed() {
        let result = fixture();
        let limits = Thresholds {
            min_download: Some(100.0),
            min_upload: Some(20.0),
            max_latency: Some(10.0),
            ..Default::default()
        };
        assert!(evaluate(&result, &limits, result.timestamp).unwrap().passed);
        let limits = Thresholds {
            max_loaded_latency: Some(50.0),
            ..Default::default()
        };
        let report = evaluate(&result, &limits, result.timestamp).unwrap();
        assert!(!report.passed);
        assert_eq!(report.checks[0].actual, None);
    }

    #[test]
    fn rejects_empty_nonfinite_negative_and_future_bypasses() {
        let result = fixture();
        assert!(evaluate(&result, &Thresholds::default(), result.timestamp).is_err());
        for limit in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(evaluate(
                &result,
                &Thresholds {
                    min_upload: Some(limit),
                    ..Default::default()
                },
                result.timestamp
            )
            .is_err());
        }
        let limits = Thresholds {
            max_age: Some(60),
            ..Default::default()
        };
        assert!(evaluate(&result, &limits, result.timestamp).unwrap().passed);
        assert!(
            !evaluate(
                &result,
                &limits,
                result.timestamp + chrono::Duration::seconds(61)
            )
            .unwrap()
            .passed
        );
        assert!(
            !evaluate(
                &result,
                &limits,
                result.timestamp - chrono::Duration::seconds(1)
            )
            .unwrap()
            .passed
        );
    }

    #[test]
    fn bounded_input_rejects_oversize_and_trailing_json() {
        assert!(read_result(std::io::repeat(b' ').take(MAX_RESULT_BYTES + 1)).is_err());
        assert!(read_result(b"{}{}".as_slice()).is_err());
    }
}
