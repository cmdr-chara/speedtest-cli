use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::model::TestResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryTrend {
    Improving,
    Stable,
    Declining,
    InsufficientData,
}

impl HistoryTrend {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Improving => "improving",
            Self::Stable => "stable",
            Self::Declining => "declining",
            Self::InsufficientData => "insufficient data",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnomalySeverity {
    Info,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryScope {
    Internet,
    Lan,
}

impl HistoryScope {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Internet => "internet",
            Self::Lan => "LAN",
        }
    }
}

impl AnomalySeverity {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warning => "WARNING",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryAnomaly {
    pub severity: AnomalySeverity,
    pub metric: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistorySummary {
    pub generated_at: chrono::DateTime<Utc>,
    pub period_days: u64,
    pub scope: HistoryScope,
    pub runs: usize,
    pub median_download_mbps: f64,
    pub best_download_mbps: f64,
    pub median_upload_mbps: f64,
    pub best_upload_mbps: f64,
    pub median_ping_ms: f64,
    pub p95_ping_ms: f64,
    pub median_quality_score: Option<f64>,
    pub s_tier_runs: usize,
    pub trend: HistoryTrend,
    pub anomalies: Vec<HistoryAnomaly>,
    pub download_sparkline: String,
}

pub fn summarize(results: &[TestResult], period_days: u64) -> Option<HistorySummary> {
    if results.is_empty() {
        return None;
    }

    // LAN throughput is a fundamentally different population from WAN tests.
    // Prefer Internet statistics whenever the period contains Internet runs;
    // otherwise summarize the available LAN history.
    let scope = if results.iter().any(|result| !is_lan(result)) {
        HistoryScope::Internet
    } else {
        HistoryScope::Lan
    };
    let results = results
        .iter()
        .filter(|result| matches_scope(result, scope))
        .cloned()
        .collect::<Vec<_>>();

    let downloads: Vec<f64> = results.iter().map(|result| result.download.mbps).collect();
    let uploads: Vec<f64> = results.iter().map(|result| result.upload.mbps).collect();
    let pings: Vec<f64> = results
        .iter()
        .map(|result| result.latency.idle_ms)
        .collect();
    let quality_scores: Vec<f64> = results
        .iter()
        .filter_map(|result| {
            result
                .analysis
                .as_ref()
                .map(|analysis| f64::from(analysis.quality.score))
        })
        .collect();
    let s_tier_runs = results
        .iter()
        .filter(|result| {
            result
                .analysis
                .as_ref()
                .is_some_and(|analysis| analysis.quality.is_s_tier())
        })
        .count();

    Some(HistorySummary {
        generated_at: Utc::now(),
        period_days,
        scope,
        runs: results.len(),
        median_download_mbps: median(&downloads),
        best_download_mbps: downloads.iter().copied().fold(0.0, f64::max),
        median_upload_mbps: median(&uploads),
        best_upload_mbps: uploads.iter().copied().fold(0.0, f64::max),
        median_ping_ms: median(&pings),
        p95_ping_ms: percentile(&pings, 0.95),
        median_quality_score: (!quality_scores.is_empty()).then(|| median(&quality_scores)),
        s_tier_runs,
        trend: download_trend(&downloads),
        anomalies: detect_latest_anomalies(&results),
        download_sparkline: sparkline(&downloads, 48),
    })
}

pub fn latest_comparable_pair(results: &[TestResult]) -> Option<(TestResult, TestResult)> {
    [false, true]
        .into_iter()
        .filter_map(|lan_scope| {
            let mut scoped = results
                .iter()
                .rev()
                .filter(|result| is_lan(result) == lan_scope);
            let after = scoped.next()?;
            let before = scoped.next()?;
            Some((before.clone(), after.clone()))
        })
        .max_by_key(|(_, after)| after.timestamp)
}

fn is_lan(result: &TestResult) -> bool {
    result.backend.eq_ignore_ascii_case("lan")
}

fn matches_scope(result: &TestResult, scope: HistoryScope) -> bool {
    match scope {
        HistoryScope::Internet => !is_lan(result),
        HistoryScope::Lan => is_lan(result),
    }
}

pub fn detect_latest_anomalies(results: &[TestResult]) -> Vec<HistoryAnomaly> {
    if results.len() < 6 {
        return Vec::new();
    }

    let latest = results.last().expect("history has at least one result");
    let baseline = &results[..results.len() - 1];
    let baseline_download = median(
        &baseline
            .iter()
            .map(|result| result.download.mbps)
            .collect::<Vec<_>>(),
    );
    let baseline_upload = median(
        &baseline
            .iter()
            .map(|result| result.upload.mbps)
            .collect::<Vec<_>>(),
    );
    let baseline_ping = median(
        &baseline
            .iter()
            .map(|result| result.latency.idle_ms)
            .collect::<Vec<_>>(),
    );

    let mut anomalies = Vec::new();
    if baseline_download > 0.0 && latest.download.mbps < baseline_download * 0.75 {
        anomalies.push(HistoryAnomaly {
            severity: AnomalySeverity::Warning,
            metric: "download".to_string(),
            message: format!(
                "Latest download {:.1} Mbps is {:.0}% below the prior median {:.1} Mbps.",
                latest.download.mbps,
                (1.0 - latest.download.mbps / baseline_download) * 100.0,
                baseline_download
            ),
        });
    }

    if baseline_upload > 0.0 && latest.upload.mbps < baseline_upload * 0.70 {
        anomalies.push(HistoryAnomaly {
            severity: AnomalySeverity::Warning,
            metric: "upload".to_string(),
            message: format!(
                "Latest upload {:.1} Mbps is {:.0}% below the prior median {:.1} Mbps.",
                latest.upload.mbps,
                (1.0 - latest.upload.mbps / baseline_upload) * 100.0,
                baseline_upload
            ),
        });
    }

    if baseline_ping > 0.0
        && latest.latency.idle_ms >= baseline_ping * 1.5
        && latest.latency.idle_ms - baseline_ping >= 5.0
    {
        anomalies.push(HistoryAnomaly {
            severity: AnomalySeverity::Warning,
            metric: "latency".to_string(),
            message: format!(
                "Latest idle latency {:.1} ms is {:.0}% above the prior median {:.1} ms.",
                latest.latency.idle_ms,
                (latest.latency.idle_ms / baseline_ping - 1.0) * 100.0,
                baseline_ping
            ),
        });
    }

    let latest_quality = latest
        .analysis
        .as_ref()
        .map(|analysis| f64::from(analysis.quality.score));
    let baseline_quality: Vec<f64> = baseline
        .iter()
        .filter_map(|result| {
            result
                .analysis
                .as_ref()
                .map(|analysis| f64::from(analysis.quality.score))
        })
        .collect();
    if let (Some(latest_quality), false) = (latest_quality, baseline_quality.is_empty()) {
        let baseline_quality = median(&baseline_quality);
        if latest_quality <= baseline_quality - 15.0 {
            anomalies.push(HistoryAnomaly {
                severity: AnomalySeverity::Warning,
                metric: "quality".to_string(),
                message: format!(
                    "Latest quality score {:.0} is {:.0} points below the prior median {:.0}.",
                    latest_quality,
                    baseline_quality - latest_quality,
                    baseline_quality
                ),
            });
        }
    }

    anomalies
}

pub fn sparkline(values: &[f64], width: usize) -> String {
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if values.is_empty() || width == 0 {
        return String::new();
    }

    let start = values.len().saturating_sub(width);
    let visible = &values[start..];
    let min = visible.iter().copied().fold(f64::INFINITY, f64::min);
    let max = visible.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = max - min;

    visible
        .iter()
        .map(|value| {
            if span <= f64::EPSILON {
                return BLOCKS[4];
            }
            let position = ((*value - min) / span * 7.0).round().clamp(0.0, 7.0) as usize;
            BLOCKS[position]
        })
        .collect()
}

fn download_trend(downloads: &[f64]) -> HistoryTrend {
    if downloads.len() < 6 {
        return HistoryTrend::InsufficientData;
    }
    let middle = downloads.len() / 2;
    let earlier = median(&downloads[..middle]);
    let recent = median(&downloads[middle..]);
    if earlier <= f64::EPSILON {
        return HistoryTrend::InsufficientData;
    }

    let change = recent / earlier - 1.0;
    if change >= 0.10 {
        HistoryTrend::Improving
    } else if change <= -0.10 {
        HistoryTrend::Declining
    } else {
        HistoryTrend::Stable
    }
}

fn median(values: &[f64]) -> f64 {
    percentile(values, 0.50)
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    if sorted.len() == 1 {
        return sorted[0];
    }
    let position = percentile.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    if lower == upper {
        return sorted[lower];
    }
    let fraction = position - lower as f64;
    sorted[lower] + (sorted[upper] - sorted[lower]) * fraction
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use crate::model::{LatencyResult, ServerInfo, ThroughputResult};

    use super::*;

    fn result(index: usize, download: f64, upload: f64, ping: f64) -> TestResult {
        TestResult {
            timestamp: Utc::now() + Duration::minutes(index as i64),
            backend: "test".into(),
            server: ServerInfo {
                host: "example.test".into(),
                name: "Example".into(),
            },
            latency: LatencyResult {
                idle_ms: ping,
                jitter_ms: 1.0,
                download_loaded_ms: None,
                upload_loaded_ms: None,
                packet_loss_percent: None,
            },
            download: ThroughputResult {
                mbps: download,
                bytes: 1,
                seconds: 1.0,
            },
            upload: ThroughputResult {
                mbps: upload,
                bytes: 1,
                seconds: 1.0,
            },
            analysis: None,
        }
    }

    #[test]
    fn flags_large_latest_download_regression() {
        let mut results: Vec<_> = (0..6)
            .map(|index| result(index, 800.0, 100.0, 10.0))
            .collect();
        results.push(result(7, 400.0, 100.0, 10.0));
        let anomalies = detect_latest_anomalies(&results);
        assert!(anomalies.iter().any(|item| item.metric == "download"));
    }

    #[test]
    fn sparkline_has_requested_visible_width() {
        let line = sparkline(&[1.0, 2.0, 3.0, 4.0, 5.0], 4);
        assert_eq!(line.chars().count(), 4);
    }

    #[test]
    fn trend_detects_improvement() {
        let summary = summarize(
            &[
                result(0, 100.0, 10.0, 20.0),
                result(1, 100.0, 10.0, 20.0),
                result(2, 100.0, 10.0, 20.0),
                result(3, 150.0, 10.0, 20.0),
                result(4, 160.0, 10.0, 20.0),
                result(5, 170.0, 10.0, 20.0),
            ],
            30,
        )
        .unwrap();
        assert_eq!(summary.trend, HistoryTrend::Improving);
    }

    #[test]
    fn internet_stats_are_not_distorted_by_lan_results() {
        let internet = [result(0, 100.0, 20.0, 10.0), result(1, 120.0, 25.0, 12.0)];
        let mut lan = result(2, 20_000.0, 18_000.0, 0.1);
        lan.backend = "lan".to_string();
        let summary = summarize(&[internet[0].clone(), internet[1].clone(), lan], 30).unwrap();

        assert_eq!(summary.scope, HistoryScope::Internet);
        assert_eq!(summary.runs, 2);
        assert_eq!(summary.median_download_mbps, 110.0);
    }

    #[test]
    fn implicit_compare_uses_the_latest_result_in_the_same_scope() {
        let wan = result(0, 100.0, 20.0, 10.0);
        let mut first_lan = result(1, 1_000.0, 900.0, 1.0);
        first_lan.backend = "lan".to_string();
        let second_wan = result(2, 120.0, 25.0, 9.0);
        let mut second_lan = result(3, 1_100.0, 950.0, 0.9);
        second_lan.backend = "lan".to_string();

        let (before, after) =
            latest_comparable_pair(&[wan, first_lan.clone(), second_wan, second_lan.clone()])
                .unwrap();
        assert_eq!(before.timestamp, first_lan.timestamp);
        assert_eq!(after.timestamp, second_lan.timestamp);
    }

    #[test]
    fn implicit_compare_uses_an_available_internet_pair_after_one_lan_run() {
        let first_wan = result(0, 100.0, 20.0, 10.0);
        let second_wan = result(1, 120.0, 25.0, 9.0);
        let mut lone_lan = result(2, 1_000.0, 900.0, 1.0);
        lone_lan.backend = "lan".to_string();

        let (before, after) =
            latest_comparable_pair(&[first_wan.clone(), second_wan.clone(), lone_lan]).unwrap();
        assert_eq!(before.timestamp, first_wan.timestamp);
        assert_eq!(after.timestamp, second_wan.timestamp);
    }
}
