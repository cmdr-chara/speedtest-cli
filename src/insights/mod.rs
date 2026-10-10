//! Offline connection intelligence. Never pool different backends or server paths.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Timelike, Utc};
use serde::Serialize;

use crate::{
    history::{self, HistoryScope, HistoryTrend},
    model::TestResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryMetric {
    Download,
    Upload,
    IdleLatency,
    Jitter,
    LoadedLatency,
    Quality,
}

impl HistoryMetric {
    pub const ALL: [Self; 6] = [
        Self::Download,
        Self::Upload,
        Self::IdleLatency,
        Self::Jitter,
        Self::LoadedLatency,
        Self::Quality,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Download => "Download",
            Self::Upload => "Upload",
            Self::IdleLatency => "Idle latency",
            Self::Jitter => "Jitter",
            Self::LoadedLatency => "Loaded latency",
            Self::Quality => "Quality",
        }
    }

    pub const fn unit(self) -> &'static str {
        match self {
            Self::Download | Self::Upload => "Mbps",
            Self::Quality => "pts",
            _ => "ms",
        }
    }

    pub const fn higher_is_better(self) -> bool {
        matches!(self, Self::Download | Self::Upload | Self::Quality)
    }

    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|metric| *metric == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    /// Missing, negative, and non-finite readings are unavailable, never zero.
    /// Worst loaded latency requires evidence from BOTH transfer directions.
    pub fn value(self, result: &TestResult) -> Option<f64> {
        let value = match self {
            Self::Download => Some(result.download.mbps),
            Self::Upload => Some(result.upload.mbps),
            Self::IdleLatency => Some(result.latency.idle_ms),
            Self::Jitter => Some(result.latency.jitter_ms),
            Self::LoadedLatency => {
                let download = valid(result.latency.download_loaded_ms)?;
                let upload = valid(result.latency.upload_loaded_ms)?;
                Some(download.max(upload))
            }
            Self::Quality => result
                .analysis
                .as_ref()
                .filter(|analysis| analysis.quality.score <= 100)
                .map(|analysis| f64::from(analysis.quality.score)),
        };
        valid(value)
    }
}

fn valid(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value >= 0.0)
}

#[derive(Debug, Clone, Serialize)]
pub struct InsightsReport {
    pub schema_version: u8,
    pub period_days: u64,
    pub runs: usize,
    pub groups: Vec<ConnectionInsights>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionInsights {
    pub backend: String,
    pub server_host: String,
    pub scope: HistoryScope,
    pub runs: usize,
    pub sampled_days: usize,
    pub first_timestamp: DateTime<Utc>,
    pub last_timestamp: DateTime<Utc>,
    pub metrics: Vec<MetricInsights>,
    pub time_of_day: Vec<TimeBucket>,
    pub time_of_day_comparison: Option<TimeOfDayComparison>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricInsights {
    pub metric: HistoryMetric,
    pub unit: &'static str,
    pub samples: usize,
    pub distribution: Option<MetricDistribution>,
    pub trend: HistoryTrend,
    pub recent_change_percent: Option<f64>,
    pub sparkline: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricDistribution {
    pub min: f64,
    pub p10: f64,
    pub median: f64,
    pub p95: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimeBucket {
    pub start_hour_utc: u32,
    pub end_hour_utc: u32,
    pub runs: usize,
    pub sampled_days: usize,
    pub download_samples: usize,
    pub median_download_mbps: Option<f64>,
    pub median_upload_mbps: Option<f64>,
    pub median_idle_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimeOfDayComparison {
    pub fastest_start_hour_utc: u32,
    pub slowest_start_hour_utc: u32,
    pub download_gap_percent: f64,
}

/// Date filtering and filesystem reads belong to storage/the caller. Sorting is
/// stable, so timestamp collisions retain their original record order.
pub fn analyze(results: &[TestResult], period_days: u64) -> InsightsReport {
    let mut groups: BTreeMap<(String, String), Vec<&TestResult>> = BTreeMap::new();
    for result in results {
        groups
            .entry((
                result.backend.to_ascii_lowercase(),
                history::server_identity(&result.server.host),
            ))
            .or_default()
            .push(result);
    }
    let groups = groups
        .into_iter()
        .map(|((backend, host), mut results)| {
            results.sort_by_key(|result| result.timestamp);
            let metrics = HistoryMetric::ALL
                .into_iter()
                .map(|metric| {
                    let values: Vec<_> = results
                        .iter()
                        .filter_map(|result| metric.value(result))
                        .collect();
                    let (trend, recent_change_percent) = trend(&values, metric.higher_is_better());
                    MetricInsights {
                        metric,
                        unit: metric.unit(),
                        samples: values.len(),
                        distribution: distribution(&values),
                        trend,
                        recent_change_percent,
                        sparkline: metric_sparkline(&results, metric, 48),
                    }
                })
                .collect();
            let time_of_day: Vec<_> = (0..4)
                .map(|slot| {
                    let start = slot * 6;
                    let samples: Vec<_> = results
                        .iter()
                        .copied()
                        .filter(|result| (start..start + 6).contains(&result.timestamp.hour()))
                        .collect();
                    let downloads: Vec<_> = samples
                        .iter()
                        .copied()
                        .filter(|result| HistoryMetric::Download.value(result).is_some())
                        .collect();
                    TimeBucket {
                        start_hour_utc: start,
                        end_hour_utc: start + 6,
                        runs: samples.len(),
                        // Eligibility counts days with usable download evidence, not just records.
                        sampled_days: distinct_days(&downloads),
                        download_samples: downloads.len(),
                        median_download_mbps: metric_median(&samples, HistoryMetric::Download),
                        median_upload_mbps: metric_median(&samples, HistoryMetric::Upload),
                        median_idle_ms: metric_median(&samples, HistoryMetric::IdleLatency),
                    }
                })
                .collect();
            let mut eligible: Vec<_> = time_of_day
                .iter()
                .filter(|bucket| bucket.download_samples >= 3 && bucket.sampled_days >= 2)
                .collect();
            eligible.sort_by(|left, right| {
                left.median_download_mbps
                    .unwrap_or(0.0)
                    .total_cmp(&right.median_download_mbps.unwrap_or(0.0))
            });
            let time_of_day_comparison = if eligible.len() >= 2 {
                let slowest = eligible[0];
                let fastest = eligible[eligible.len() - 1];
                let fast = fastest.median_download_mbps.unwrap_or(0.0);
                let slow = slowest.median_download_mbps.unwrap_or(0.0);
                (fast > 0.0).then(|| TimeOfDayComparison {
                    fastest_start_hour_utc: fastest.start_hour_utc,
                    slowest_start_hour_utc: slowest.start_hour_utc,
                    download_gap_percent: (1.0 - slow / fast) * 100.0,
                })
            } else {
                None
            };
            ConnectionInsights {
                backend,
                server_host: host,
                scope: if history::is_lan(results[0]) {
                    HistoryScope::Lan
                } else {
                    HistoryScope::Internet
                },
                runs: results.len(),
                sampled_days: distinct_days(&results),
                first_timestamp: results[0].timestamp,
                last_timestamp: results[results.len() - 1].timestamp,
                metrics,
                time_of_day,
                time_of_day_comparison,
            }
        })
        .collect();
    InsightsReport {
        schema_version: 1,
        period_days,
        runs: results.len(),
        groups,
    }
}

fn distinct_days(results: &[&TestResult]) -> usize {
    results
        .iter()
        .map(|result| result.timestamp.date_naive())
        .collect::<BTreeSet<_>>()
        .len()
}

fn metric_median(results: &[&TestResult], metric: HistoryMetric) -> Option<f64> {
    distribution(
        &results
            .iter()
            .filter_map(|result| metric.value(result))
            .collect::<Vec<_>>(),
    )
    .map(|stats| stats.median)
}

pub fn distribution(values: &[f64]) -> Option<MetricDistribution> {
    let mut sorted: Vec<_> = values
        .iter()
        .copied()
        .filter(|value| valid(Some(*value)).is_some())
        .collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    let quantile = |fraction: f64| {
        let position = (sorted.len() - 1) as f64 * fraction;
        let lower = sorted[position.floor() as usize];
        let upper = sorted[position.ceil() as usize];
        // Interpolate the bounded difference so two very large values do not
        // overflow when their weighted sum is rounded.
        lower + (upper - lower) * position.fract()
    };
    Some(MetricDistribution {
        min: sorted[0],
        p10: quantile(0.10),
        median: quantile(0.50),
        p95: quantile(0.95),
        max: sorted[sorted.len() - 1],
    })
}

fn trend(values: &[f64], higher: bool) -> (HistoryTrend, Option<f64>) {
    if values.len() < 6 {
        return (HistoryTrend::InsufficientData, None);
    }
    let middle = values.len() / 2;
    let before = distribution(&values[..middle]).map_or(0.0, |stats| stats.median);
    let after = distribution(&values[middle..]).map_or(0.0, |stats| stats.median);
    if before <= f64::EPSILON {
        return (HistoryTrend::InsufficientData, None);
    }
    let change = (after / before - 1.0) * 100.0;
    if !change.is_finite() {
        return (HistoryTrend::InsufficientData, None);
    }
    let desirable = if higher { change } else { -change };
    let trend = if desirable >= 10.0 {
        HistoryTrend::Improving
    } else if desirable <= -10.0 {
        HistoryTrend::Declining
    } else {
        HistoryTrend::Stable
    };
    (trend, Some(change))
}

fn metric_sparkline(results: &[&TestResult], metric: HistoryMetric, width: usize) -> String {
    let values: Vec<_> = results
        .iter()
        .skip(results.len().saturating_sub(width))
        .map(|result| metric.value(result))
        .collect();
    let present: Vec<_> = values.iter().copied().flatten().collect();
    let line = history::sparkline(&present, width);
    let mut blocks = line.chars();
    values
        .iter()
        .map(|value| {
            if value.is_some() {
                blocks.next().unwrap_or('·')
            } else {
                '·'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Duration, TimeZone};

    fn result(day: i64, hour: u32, download: f64) -> TestResult {
        let mut result: TestResult =
            serde_json::from_str(include_str!("../../tests/fixtures/result.json")).unwrap();
        result.timestamp =
            Utc.with_ymd_and_hms(2026, 9, 1, hour, 0, 0).unwrap() + Duration::days(day);
        result.download.mbps = download;
        result
    }

    #[test]
    fn empty_report_has_a_versioned_stable_shape() {
        let report = analyze(&[], 30);
        assert_eq!(
            serde_json::to_value(report).unwrap(),
            serde_json::json!({
                "schema_version": 1, "period_days": 30, "runs": 0, "groups": []
            })
        );
    }

    #[test]
    fn keeps_backends_servers_and_lan_in_separate_populations() {
        let first = result(0, 9, 100.0);
        let mut other_server = first.clone();
        other_server.server.host = "another.test".into();
        let mut lan = first.clone();
        lan.backend = "lan".into();
        lan.download.mbps = 20_000.0;
        let mut other_backend = first.clone();
        other_backend.backend = "librespeed".into();
        let report = analyze(&[first, other_server, lan, other_backend], 30);
        assert_eq!(report.groups.len(), 4);
        assert!(report.groups.iter().all(|group| group.runs == 1));
        assert_eq!(
            report
                .groups
                .iter()
                .find(|group| group.backend == "lan")
                .unwrap()
                .scope,
            HistoryScope::Lan
        );
    }

    #[test]
    fn url_groups_normalize_hosts_without_merging_case_sensitive_paths() {
        let mut samples = Vec::new();
        for host in [
            "HTTPS://EXAMPLE.TEST/A/",
            "https://example.test:443/A/",
            "https://example.test/a/",
            "https://example.test/A/?Token=One",
            "https://example.test/A/?Token=one",
        ] {
            let mut sample = result(0, 9, 100.0);
            sample.server.host = host.into();
            samples.push(sample);
        }
        let report = analyze(&samples, 30);
        assert_eq!(report.groups.len(), 4);
        let upper = report
            .groups
            .iter()
            .find(|group| group.server_host == "https://example.test/A/")
            .unwrap();
        assert_eq!(upper.runs, 2);
        assert_eq!(samples[0].server.host, "HTTPS://EXAMPLE.TEST/A/");
    }

    #[test]
    fn orders_history_before_computing_direction_aware_trends() {
        let mut results: Vec<_> = (0..6)
            .map(|day| result(day, 9, if day < 3 { 100.0 } else { 150.0 }))
            .collect();
        for (index, result) in results.iter_mut().enumerate() {
            result.latency.idle_ms = if index < 3 { 10.0 } else { 30.0 };
        }
        results.reverse();
        let report = analyze(&results, 30);
        let group = &report.groups[0];
        assert!(group.first_timestamp < group.last_timestamp);
        assert_eq!(group.sampled_days, 6);
        assert_eq!(group.metrics[0].trend, HistoryTrend::Improving);
        assert_eq!(group.metrics[2].trend, HistoryTrend::Declining);
        assert_eq!(group.metrics[0].recent_change_percent, Some(50.0));
    }

    #[test]
    fn missing_invalid_and_partial_coverage_never_become_zeroes() {
        let mut results = vec![
            result(0, 9, 100.0),
            result(1, 9, -1.0),
            result(2, 9, f64::NAN),
        ];
        results[0].latency.download_loaded_ms = None;
        results[1].latency.upload_loaded_ms = Some(f64::INFINITY);
        let report = analyze(&results, 30);
        let metrics = &report.groups[0].metrics;
        assert_eq!(metrics[0].samples, 1);
        assert_eq!(metrics[0].distribution.as_ref().unwrap().median, 100.0);
        assert_eq!(
            metrics[0].sparkline.chars().skip(1).collect::<String>(),
            "··"
        );
        assert_eq!(metrics[4].samples, 1);
        assert_eq!(metrics[5].samples, 0);
        assert!(metrics[5].distribution.is_none());
        assert_eq!(metrics[5].sparkline, "···");
    }

    #[test]
    fn quantiles_are_interpolated_and_do_not_overflow() {
        let stats = distribution(&[0.0, 10.0, 20.0, 30.0, 40.0]).unwrap();
        assert_eq!(stats.p10, 4.0);
        assert_eq!(stats.median, 20.0);
        assert!((stats.p95 - 38.0).abs() < 0.0001);
        assert!(distribution(&[f64::MAX, f64::MAX])
            .unwrap()
            .median
            .is_finite());
        assert!(distribution(&[]).is_none());
    }

    #[test]
    fn time_of_day_requires_repeated_evidence_on_multiple_days() {
        let mut results = Vec::new();
        for day in 0..3 {
            results.push(result(day, 9, 100.0));
            results.push(result(day, 19, 50.0));
        }
        let report = analyze(&results, 30);
        let group = &report.groups[0];
        let comparison = group.time_of_day_comparison.as_ref().unwrap();
        assert_eq!(comparison.fastest_start_hour_utc, 6);
        assert_eq!(comparison.slowest_start_hour_utc, 18);
        assert_eq!(comparison.download_gap_percent, 50.0);
        assert_eq!(group.time_of_day[0].median_download_mbps, None);
        assert_eq!(group.time_of_day[1].sampled_days, 3);
        for result in &mut results {
            result.timestamp = result.timestamp - Duration::days((result.timestamp.day0()) as i64);
        }
        assert!(analyze(&results, 30).groups[0]
            .time_of_day_comparison
            .is_none());
    }

    #[test]
    fn zero_baseline_and_single_runs_do_not_invent_a_trend() {
        let results: Vec<_> = (0..6)
            .map(|day| result(day, 9, if day < 3 { 0.0 } else { 100.0 }))
            .collect();
        assert_eq!(
            analyze(&results, 30).groups[0].metrics[0].trend,
            HistoryTrend::InsufficientData
        );
        assert_eq!(
            analyze(&results[..1], 30).groups[0].metrics[0].recent_change_percent,
            None
        );
    }
}
