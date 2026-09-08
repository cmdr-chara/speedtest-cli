use serde::{Deserialize, Serialize};

use crate::model::TestResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricDelta {
    pub before: f64,
    pub after: f64,
    pub absolute_change: f64,
    pub percent_change: Option<f64>,
    pub improved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionalMetricDelta {
    pub before: Option<f64>,
    pub after: Option<f64>,
    pub absolute_change: Option<f64>,
    pub percent_change: Option<f64>,
    pub improved: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareResult {
    pub before_timestamp: chrono::DateTime<chrono::Utc>,
    pub after_timestamp: chrono::DateTime<chrono::Utc>,
    pub download_mbps: MetricDelta,
    pub upload_mbps: MetricDelta,
    pub ping_ms: MetricDelta,
    pub jitter_ms: MetricDelta,
    pub quality_score: OptionalMetricDelta,
    pub bufferbloat_ms: OptionalMetricDelta,
    pub verdict: String,
    pub highlight: String,
}

pub fn compare(before: &TestResult, after: &TestResult) -> CompareResult {
    let quality_before = before
        .analysis
        .as_ref()
        .map(|analysis| f64::from(analysis.quality.score));
    let quality_after = after
        .analysis
        .as_ref()
        .map(|analysis| f64::from(analysis.quality.score));
    let buffer_before = before
        .analysis
        .as_ref()
        .and_then(|analysis| analysis.quality.bufferbloat.worst_increase_ms);
    let buffer_after = after
        .analysis
        .as_ref()
        .and_then(|analysis| analysis.quality.bufferbloat.worst_increase_ms);

    let download = higher_is_better(before.download.mbps, after.download.mbps);
    let upload = higher_is_better(before.upload.mbps, after.upload.mbps);
    let ping = lower_is_better(before.latency.idle_ms, after.latency.idle_ms);
    let jitter = lower_is_better(before.latency.jitter_ms, after.latency.jitter_ms);
    let quality = optional_higher_is_better(quality_before, quality_after);
    let bufferbloat = optional_lower_is_better(buffer_before, buffer_after);

    let mut votes = 0_i32;
    votes += significant_vote(&download, 5.0, 1);
    votes += significant_vote(&upload, 5.0, 1);
    votes += significant_vote(&ping, 10.0, 1);
    votes += significant_vote(&jitter, 10.0, 1);
    if let (Some(before), Some(after)) = (quality_before, quality_after) {
        if after >= before + 5.0 {
            votes += 2;
        } else if after <= before - 5.0 {
            votes -= 2;
        }
    }
    if let (Some(before), Some(after)) = (buffer_before, buffer_after) {
        if after <= before - 10.0 {
            votes += 1;
        } else if after >= before + 10.0 {
            votes -= 1;
        }
    }

    let verdict = if votes >= 3 {
        "after is substantially better"
    } else if votes <= -3 {
        "before is substantially better"
    } else if votes > 0 {
        "after is slightly better overall"
    } else if votes < 0 {
        "before is slightly better overall"
    } else {
        "results are mixed or effectively tied"
    }
    .to_string();

    let highlight = strongest_highlight(&download, &upload, &ping, &jitter, &quality, &bufferbloat);

    CompareResult {
        before_timestamp: before.timestamp,
        after_timestamp: after.timestamp,
        download_mbps: download,
        upload_mbps: upload,
        ping_ms: ping,
        jitter_ms: jitter,
        quality_score: quality,
        bufferbloat_ms: bufferbloat,
        verdict,
        highlight,
    }
}

fn higher_is_better(before: f64, after: f64) -> MetricDelta {
    metric_delta(before, after, after > before)
}

fn lower_is_better(before: f64, after: f64) -> MetricDelta {
    metric_delta(before, after, after < before)
}

fn metric_delta(before: f64, after: f64, improved: bool) -> MetricDelta {
    MetricDelta {
        before,
        after,
        absolute_change: after - before,
        percent_change: percent_change(before, after),
        improved,
    }
}

fn optional_higher_is_better(before: Option<f64>, after: Option<f64>) -> OptionalMetricDelta {
    optional_metric_delta(before, after, |before, after| after > before)
}

fn optional_lower_is_better(before: Option<f64>, after: Option<f64>) -> OptionalMetricDelta {
    optional_metric_delta(before, after, |before, after| after < before)
}

fn optional_metric_delta(
    before: Option<f64>,
    after: Option<f64>,
    improved: impl FnOnce(f64, f64) -> bool,
) -> OptionalMetricDelta {
    let (absolute_change, percent_change, improved) = match (before, after) {
        (Some(before), Some(after)) => (
            Some(after - before),
            percent_change(before, after),
            Some(improved(before, after)),
        ),
        _ => (None, None, None),
    };
    OptionalMetricDelta {
        before,
        after,
        absolute_change,
        percent_change,
        improved,
    }
}

fn percent_change(before: f64, after: f64) -> Option<f64> {
    (before.abs() > f64::EPSILON).then(|| (after / before - 1.0) * 100.0)
}

fn significant_vote(metric: &MetricDelta, threshold_percent: f64, weight: i32) -> i32 {
    let significant = match metric.percent_change {
        Some(change) => change.abs() >= threshold_percent,
        None => metric.absolute_change.abs() > f64::EPSILON,
    };
    if !significant {
        0
    } else if metric.improved {
        weight
    } else {
        -weight
    }
}

fn strongest_highlight(
    download: &MetricDelta,
    upload: &MetricDelta,
    ping: &MetricDelta,
    jitter: &MetricDelta,
    quality: &OptionalMetricDelta,
    bufferbloat: &OptionalMetricDelta,
) -> String {
    let mut candidates = Vec::new();
    push_percent_highlight(&mut candidates, "download", download, true);
    push_percent_highlight(&mut candidates, "upload", upload, true);
    push_percent_highlight(&mut candidates, "ping", ping, false);
    push_percent_highlight(&mut candidates, "jitter", jitter, false);

    if let (Some(change), Some(improved)) = (quality.absolute_change, quality.improved) {
        if change.abs() > f64::EPSILON {
            candidates.push((
                change.abs() * 4.0,
                format!(
                    "quality score {} by {:.0} points",
                    if improved { "improved" } else { "fell" },
                    change.abs()
                ),
            ));
        }
    }
    if let (Some(change), Some(improved)) = (bufferbloat.absolute_change, bufferbloat.improved) {
        if change.abs() > f64::EPSILON {
            candidates.push((
                change.abs(),
                format!(
                    "bufferbloat {} by {:.1} ms",
                    if improved { "decreased" } else { "increased" },
                    change.abs()
                ),
            ));
        }
    }

    candidates
        .into_iter()
        .max_by(|left, right| left.0.total_cmp(&right.0))
        .map_or_else(|| "no dominant change".to_string(), |(_, message)| message)
}

fn push_percent_highlight(
    candidates: &mut Vec<(f64, String)>,
    label: &str,
    metric: &MetricDelta,
    higher: bool,
) {
    if metric.absolute_change.abs() <= f64::EPSILON {
        return;
    }
    if let Some(change) = metric.percent_change {
        let direction = if change >= 0.0 {
            "increased"
        } else {
            "decreased"
        };
        let desirable = if higher { change >= 0.0 } else { change <= 0.0 };
        candidates.push((
            change.abs(),
            format!(
                "{label} {direction} by {:.0}% ({})",
                change.abs(),
                if desirable { "better" } else { "worse" }
            ),
        ));
    } else {
        let increased = metric.absolute_change > 0.0;
        let direction = if increased { "increased" } else { "decreased" };
        let desirable = if higher { increased } else { !increased };
        candidates.push((
            metric.absolute_change.abs(),
            format!(
                "{label} {direction} from {:.1} to {:.1} ({})",
                metric.before,
                metric.after,
                if desirable { "better" } else { "worse" }
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::model::{LatencyResult, ServerInfo, TestResult, ThroughputResult};

    use super::*;

    fn result(download: f64, upload: f64, ping: f64, jitter: f64) -> TestResult {
        TestResult {
            timestamp: Utc::now(),
            backend: "test".to_string(),
            server: ServerInfo {
                host: "example.test".to_string(),
                name: "Example".to_string(),
            },
            latency: LatencyResult {
                idle_ms: ping,
                jitter_ms: jitter,
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
    fn recognizes_clear_after_improvement() {
        let before = result(500.0, 50.0, 20.0, 8.0);
        let after = result(800.0, 80.0, 10.0, 3.0);
        let comparison = compare(&before, &after);
        assert!(comparison.verdict.contains("after"));
        assert!(comparison.download_mbps.improved);
        assert!(comparison.ping_ms.improved);
    }

    #[test]
    fn zero_baseline_improvement_votes_for_after() {
        let before = result(0.0, 50.0, 20.0, 8.0);
        let after = result(25.0, 50.0, 20.0, 8.0);

        let comparison = compare(&before, &after);

        assert!(comparison.download_mbps.improved);
        assert_eq!(comparison.download_mbps.percent_change, None);
        assert_eq!(comparison.verdict, "after is slightly better overall");
        assert_eq!(
            comparison.highlight,
            "download increased from 0.0 to 25.0 (better)"
        );
    }

    #[test]
    fn equal_results_have_no_dominant_change() {
        let before = result(100.0, 50.0, 20.0, 8.0);
        let after = before.clone();

        let comparison = compare(&before, &after);

        assert_eq!(comparison.verdict, "results are mixed or effectively tied");
        assert_eq!(comparison.highlight, "no dominant change");
        assert!(!comparison.download_mbps.improved);
        assert!(!comparison.ping_ms.improved);
    }
}
