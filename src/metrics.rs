//! Offline Prometheus text exposition. No probes or persistence occur here.
//!
//! https://prometheus.io/docs/instrumenting/exposition_formats/
//! Metric names use base units; the canonical JSON result remains unchanged.
use std::fmt::Write;

use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};

use crate::model::TestResult;

/// Fail before publishing a stale or future-dated result. Without this explicit
/// guard, consumers can use the timestamp gauge for their own freshness policy.
pub fn ensure_fresh(result: &TestResult, max_age: u64, now: DateTime<Utc>) -> Result<()> {
    ensure!(max_age > 0, "maximum result age must be greater than zero");
    ensure!(result.timestamp <= now, "result timestamp is in the future");
    let age = now.signed_duration_since(result.timestamp);
    let seconds = age.num_seconds() as u64;
    ensure!(
        seconds < max_age || (seconds == max_age && age.subsec_nanos() == 0),
        "result is older than the maximum age of {max_age} seconds"
    );
    Ok(())
}

pub fn render(result: &TestResult) -> Result<String> {
    result.validate().context("invalid metrics result")?;
    let labels = format!(
        "backend=\"{}\",server=\"{}\"",
        escape_label(&result.backend),
        escape_label(&result.server.host),
    );
    let mut output = String::new();
    for (name, help, value) in [
        (
            "speedtest_result_timestamp_seconds",
            "Time of the saved measurement as Unix seconds, not the export time.",
            result.timestamp.timestamp() as f64
                + f64::from(result.timestamp.timestamp_subsec_nanos()) / 1_000_000_000.0,
        ),
        (
            "speedtest_download_bits_per_second",
            "Download throughput in decimal bits per second.",
            result.download.mbps * 1_000_000.0,
        ),
        (
            "speedtest_upload_bits_per_second",
            "Upload throughput in decimal bits per second.",
            result.upload.mbps * 1_000_000.0,
        ),
        (
            "speedtest_idle_latency_seconds",
            "Idle latency in seconds.",
            result.latency.idle_ms / 1_000.0,
        ),
        (
            "speedtest_jitter_seconds",
            "Idle jitter in seconds.",
            result.latency.jitter_ms / 1_000.0,
        ),
        (
            "speedtest_download_bytes",
            "Bytes measured during the saved download phase; not a cumulative counter.",
            result.download.bytes as f64,
        ),
        (
            "speedtest_upload_bytes",
            "Bytes measured during the saved upload phase; not a cumulative counter.",
            result.upload.bytes as f64,
        ),
        (
            "speedtest_download_duration_seconds",
            "Duration of the saved download phase in seconds.",
            result.download.seconds,
        ),
        (
            "speedtest_upload_duration_seconds",
            "Duration of the saved upload phase in seconds.",
            result.upload.seconds,
        ),
    ] {
        gauge(&mut output, name, help, &labels, value)?;
    }
    for (name, help, value) in [
        (
            "speedtest_download_loaded_latency_seconds",
            "Latency during download in seconds; omitted when unavailable.",
            result
                .latency
                .download_loaded_ms
                .map(|value| value / 1_000.0),
        ),
        (
            "speedtest_upload_loaded_latency_seconds",
            "Latency during upload in seconds; omitted when unavailable.",
            result.latency.upload_loaded_ms.map(|value| value / 1_000.0),
        ),
        (
            "speedtest_packet_loss_ratio",
            "Packet loss as a ratio from zero to one; omitted when unavailable.",
            result
                .latency
                .packet_loss_percent
                .map(|value| value / 100.0),
        ),
        (
            "speedtest_quality_score_ratio",
            "Quality score as a ratio from zero to one; omitted when unavailable.",
            result
                .analysis
                .as_ref()
                .map(|analysis| f64::from(analysis.quality.score) / 100.0),
        ),
    ] {
        if let Some(value) = value {
            gauge(&mut output, name, help, &labels, value)?;
        }
    }
    Ok(output)
}

fn gauge(output: &mut String, name: &str, help: &str, labels: &str, value: f64) -> Result<()> {
    ensure!(
        value.is_finite(),
        "metric {name} cannot be represented in base units"
    );
    writeln!(output, "# HELP {name} {help}")?;
    writeln!(output, "# TYPE {name} gauge")?;
    writeln!(output, "{name}{{{labels}}} {value}")?;
    Ok(())
}

fn escape_label(value: &str) -> String {
    // Keep output safe for redirected files and terminals identically. The three
    // Prometheus escape sequences preserve quotes, backslashes, and newlines.
    crate::output::safe_text(value)
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> TestResult {
        serde_json::from_str(include_str!("../tests/fixtures/result.json")).unwrap()
    }

    #[test]
    fn exposes_base_units_and_omits_absent_metrics() {
        let mut result = fixture();
        result.download.mbps = 123.5;
        result.latency.idle_ms = 12.5;
        result.latency.download_loaded_ms = Some(25.0);
        result.latency.upload_loaded_ms = None;
        result.latency.packet_loss_percent = Some(2.5);
        result.analysis = None;
        let rendered = render(&result).unwrap();
        let sample = |name: &str| {
            rendered
                .lines()
                .find(|line| line.starts_with(&format!("{name}{{")))
                .unwrap()
                .split_whitespace()
                .last()
                .unwrap()
                .parse::<f64>()
                .unwrap()
        };
        assert_eq!(sample("speedtest_download_bits_per_second"), 123_500_000.0);
        assert_eq!(sample("speedtest_idle_latency_seconds"), 0.0125);
        assert_eq!(sample("speedtest_download_loaded_latency_seconds"), 0.025);
        assert_eq!(sample("speedtest_packet_loss_ratio"), 0.025);
        assert_eq!(
            sample("speedtest_download_bytes"),
            result.download.bytes as f64
        );
        assert!(!rendered.contains("speedtest_upload_loaded_latency_seconds"));
        assert!(!rendered.contains("speedtest_quality_score_ratio"));
        assert!(rendered.ends_with('\n'));
        let mut names = std::collections::BTreeSet::new();
        for line in rendered.lines().filter(|line| !line.starts_with('#')) {
            let name = line.split('{').next().unwrap();
            assert!(names.insert(name));
            assert!(rendered.contains(&format!("# TYPE {name} gauge\n{line}\n")));
        }
    }

    #[test]
    fn labels_cannot_inject_samples_or_terminal_controls() {
        let mut result = fixture();
        result.backend = "fixture\"\\\nother_metric 1\x1b".into();
        result.server.host = "host\"\\\n".into();
        let rendered = render(&result).unwrap();
        assert!(rendered.contains("backend=\"fixture\\\"\\\\\\nother_metric 1\""));
        assert!(rendered.contains("server=\"host\\\"\\\\\\n\""));
        assert!(!rendered.contains('\x1b'));
        assert!(!rendered
            .lines()
            .any(|line| line.starts_with("other_metric")));
    }

    #[test]
    fn freshness_handles_exact_boundaries_future_dates_and_large_limits() {
        let result = fixture();
        let boundary = result.timestamp + chrono::Duration::seconds(60);
        assert!(ensure_fresh(&result, 60, boundary).is_ok());
        assert!(ensure_fresh(&result, 60, boundary + chrono::Duration::nanoseconds(1)).is_err());
        assert!(ensure_fresh(
            &result,
            60,
            result.timestamp - chrono::Duration::nanoseconds(1)
        )
        .is_err());
        assert!(ensure_fresh(&result, u64::MAX, boundary).is_ok());
        assert!(ensure_fresh(&result, 0, boundary).is_err());
    }

    #[test]
    fn invalid_values_and_conversion_overflow_fail_before_export() {
        let mut result = fixture();
        result.download.mbps = f64::MAX;
        assert!(render(&result).is_err());
        result.download.mbps = -1.0;
        assert!(render(&result).is_err());
    }
}
