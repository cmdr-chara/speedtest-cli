use crate::model::{
    BufferbloatAssessment, DiagnosticFinding, FindingSeverity, LatencyAnalysis,
    LatencyDistribution, LatencyResult, NetworkAnalysis, NetworkQuality, QualityConfidence,
    QualityGrade, ThroughputResult, WorkloadGrades,
};

pub fn summarize_latency(
    idle_samples: &[f64],
    download_loaded_samples: &[f64],
    upload_loaded_samples: &[f64],
    packet_loss_percent: Option<f64>,
) -> LatencyResult {
    let idle = distribution(idle_samples).unwrap_or_default();
    let jitter_values = consecutive_deltas(idle_samples);
    let jitter_ms = mean(&jitter_values).unwrap_or(0.0);
    let download_loaded_ms = distribution(download_loaded_samples).map(|stats| stats.median_ms);
    let upload_loaded_ms = distribution(upload_loaded_samples).map(|stats| stats.median_ms);

    LatencyResult {
        idle_ms: idle.median_ms,
        jitter_ms,
        download_loaded_ms,
        upload_loaded_ms,
        packet_loss_percent,
    }
}

pub fn build_network_analysis(
    idle_samples: &[f64],
    download_loaded_samples: &[f64],
    upload_loaded_samples: &[f64],
    latency: &LatencyResult,
    download: &ThroughputResult,
    upload: &ThroughputResult,
) -> NetworkAnalysis {
    let jitter_values = consecutive_deltas(idle_samples);
    let latency_analysis = LatencyAnalysis {
        idle: distribution(idle_samples).unwrap_or_default(),
        jitter: distribution(&jitter_values),
        download_loaded: distribution(download_loaded_samples),
        upload_loaded: distribution(upload_loaded_samples),
    };
    let quality = assess_quality(&latency_analysis, latency, download, upload);

    NetworkAnalysis {
        latency: latency_analysis,
        quality,
    }
}

pub fn distribution(values: &[f64]) -> Option<LatencyDistribution> {
    if values.is_empty() {
        return None;
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);

    Some(LatencyDistribution {
        samples: sorted.len(),
        min_ms: sorted[0],
        median_ms: percentile_sorted(&sorted, 0.50),
        p95_ms: percentile_sorted(&sorted, 0.95),
        p99_ms: percentile_sorted(&sorted, 0.99),
        max_ms: *sorted.last().expect("non-empty latency sample set"),
    })
}

fn percentile_sorted(sorted: &[f64], percentile: f64) -> f64 {
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

fn consecutive_deltas(values: &[f64]) -> Vec<f64> {
    values
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .collect()
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn assess_quality(
    analysis: &LatencyAnalysis,
    latency: &LatencyResult,
    download: &ThroughputResult,
    upload: &ThroughputResult,
) -> NetworkQuality {
    let download_increase_ms = latency
        .download_loaded_ms
        .map(|loaded| (loaded - latency.idle_ms).max(0.0));
    let upload_increase_ms = latency
        .upload_loaded_ms
        .map(|loaded| (loaded - latency.idle_ms).max(0.0));
    let worst_increase_ms = max_option(download_increase_ms, upload_increase_ms);
    let bufferbloat_grade = worst_increase_ms.map(bufferbloat_grade);

    let jitter_p95 = analysis
        .jitter
        .as_ref()
        .map_or(latency.jitter_ms, |stats| stats.p95_ms);
    let idle_p99 = analysis.idle.p99_ms;

    let mut overall_components = vec![
        (lower_is_better(latency.idle_ms, &IDLE_BANDS), 0.20),
        (lower_is_better(jitter_p95, &JITTER_BANDS), 0.15),
        (lower_is_better(idle_p99, &TAIL_BANDS), 0.10),
        (higher_is_better(download.mbps, &DOWNLOAD_BANDS), 0.15),
        (higher_is_better(upload.mbps, &UPLOAD_BANDS), 0.10),
    ];
    if let Some(increase) = worst_increase_ms {
        overall_components.push((lower_is_better(increase, &BUFFERBLOAT_BANDS), 0.30));
    }

    let base_score = weighted_score(&overall_components);
    let score = apply_packet_loss_penalty(base_score, latency.packet_loss_percent);
    let score = score.round().clamp(0.0, 100.0) as u8;
    let grade = grade_for_score(score as f64);

    let workloads = workload_grades(
        latency,
        jitter_p95,
        download_increase_ms,
        upload_increase_ms,
        download.mbps,
        upload.mbps,
    );
    let confidence = confidence(analysis);
    let bufferbloat = BufferbloatAssessment {
        download_increase_ms,
        upload_increase_ms,
        worst_increase_ms,
        grade: bufferbloat_grade,
    };
    let findings = diagnostic_findings(
        analysis,
        latency,
        download,
        upload,
        &bufferbloat,
        jitter_p95,
    );

    NetworkQuality {
        score,
        grade,
        confidence,
        bufferbloat,
        workloads,
        findings,
    }
}

fn workload_grades(
    latency: &LatencyResult,
    jitter_p95: f64,
    download_increase_ms: Option<f64>,
    upload_increase_ms: Option<f64>,
    download_mbps: f64,
    upload_mbps: f64,
) -> WorkloadGrades {
    let mut gaming = vec![
        (lower_is_better(latency.idle_ms, &IDLE_BANDS), 0.45),
        (lower_is_better(jitter_p95, &JITTER_BANDS), 0.30),
    ];
    if let Some(increase) = max_option(download_increase_ms, upload_increase_ms) {
        gaming.push((lower_is_better(increase, &BUFFERBLOAT_BANDS), 0.25));
    }

    let mut video_calls = vec![
        (lower_is_better(latency.idle_ms, &VIDEO_LATENCY_BANDS), 0.20),
        (lower_is_better(jitter_p95, &VIDEO_JITTER_BANDS), 0.25),
        (higher_is_better(upload_mbps, &VIDEO_UPLOAD_BANDS), 0.30),
    ];
    if let Some(increase) = upload_increase_ms {
        video_calls.push((lower_is_better(increase, &VIDEO_LOADED_BANDS), 0.25));
    }

    let mut streaming = vec![
        (
            higher_is_better(download_mbps, &STREAMING_DOWNLOAD_BANDS),
            0.75,
        ),
        (lower_is_better(jitter_p95, &STREAMING_JITTER_BANDS), 0.10),
    ];
    if let Some(increase) = download_increase_ms {
        streaming.push((lower_is_better(increase, &STREAMING_LOADED_BANDS), 0.15));
    }

    let mut cloud_gaming = vec![
        (lower_is_better(latency.idle_ms, &CLOUD_LATENCY_BANDS), 0.35),
        (lower_is_better(jitter_p95, &CLOUD_JITTER_BANDS), 0.25),
        (higher_is_better(download_mbps, &CLOUD_DOWNLOAD_BANDS), 0.15),
    ];
    if let Some(increase) = download_increase_ms {
        cloud_gaming.push((lower_is_better(increase, &CLOUD_LOADED_BANDS), 0.25));
    }

    WorkloadGrades {
        gaming: grade_for_score(apply_packet_loss_penalty(
            weighted_score(&gaming),
            latency.packet_loss_percent,
        )),
        video_calls: grade_for_score(apply_packet_loss_penalty(
            weighted_score(&video_calls),
            latency.packet_loss_percent,
        )),
        streaming: grade_for_score(weighted_score(&streaming)),
        cloud_gaming: grade_for_score(apply_packet_loss_penalty(
            weighted_score(&cloud_gaming),
            latency.packet_loss_percent,
        )),
    }
}

fn confidence(analysis: &LatencyAnalysis) -> QualityConfidence {
    let idle_samples = analysis.idle.samples;
    let download_samples = analysis
        .download_loaded
        .as_ref()
        .map_or(0, |stats| stats.samples);
    let upload_samples = analysis
        .upload_loaded
        .as_ref()
        .map_or(0, |stats| stats.samples);

    if idle_samples >= 20 && download_samples >= 8 && upload_samples >= 8 {
        QualityConfidence::High
    } else if idle_samples >= 12 && (download_samples >= 4 || upload_samples >= 4) {
        QualityConfidence::Moderate
    } else {
        QualityConfidence::Limited
    }
}

fn diagnostic_findings(
    analysis: &LatencyAnalysis,
    latency: &LatencyResult,
    download: &ThroughputResult,
    upload: &ThroughputResult,
    bufferbloat: &BufferbloatAssessment,
    jitter_p95: f64,
) -> Vec<DiagnosticFinding> {
    let mut findings = Vec::new();

    if let Some(worst) = bufferbloat.worst_increase_ms {
        if worst >= 30.0 {
            let download_increase = bufferbloat.download_increase_ms.unwrap_or_default();
            let upload_increase = bufferbloat.upload_increase_ms.unwrap_or_default();
            let (direction, loaded_ms) = if upload_increase >= download_increase {
                (
                    "upload",
                    latency.upload_loaded_ms.unwrap_or(latency.idle_ms),
                )
            } else {
                (
                    "download",
                    latency.download_loaded_ms.unwrap_or(latency.idle_ms),
                )
            };
            let severity = if worst >= 120.0 {
                FindingSeverity::Critical
            } else {
                FindingSeverity::Warning
            };
            let title = if worst >= 120.0 {
                format!("Severe {direction} bufferbloat")
            } else if worst >= 60.0 {
                format!("High {direction} bufferbloat")
            } else {
                format!("Moderate {direction} queueing")
            };
            let relative = if latency.idle_ms > f64::EPSILON {
                worst / latency.idle_ms * 100.0
            } else {
                0.0
            };
            let recommendation = if direction == "upload" {
                "Enable SQM/CAKE/FQ-CoDel if your router supports it, or shape upstream traffic to roughly 90–95% of measured upload capacity."
            } else {
                "Enable SQM/CAKE/FQ-CoDel if available and check for competing downloads; shaping slightly below line rate can reduce queueing."
            };
            findings.push(DiagnosticFinding {
                severity,
                title,
                evidence: format!(
                    "Median latency rose from {:.1} ms idle to {:.1} ms under {direction} load (+{worst:.1} ms, +{relative:.0}%).",
                    latency.idle_ms, loaded_ms
                ),
                recommendation: Some(recommendation.to_string()),
            });
        }
    } else {
        findings.push(DiagnosticFinding {
            severity: FindingSeverity::Warning,
            title: "Loaded-latency coverage is incomplete".to_string(),
            evidence: "The test could not collect enough loaded-latency probes to grade bufferbloat.".to_string(),
            recommendation: Some(
                "Retry later or use --streams 1 if the public test endpoint is throttling requests."
                    .to_string(),
            ),
        });
    }

    if jitter_p95 >= 15.0 {
        findings.push(DiagnosticFinding {
            severity: if jitter_p95 >= 40.0 {
                FindingSeverity::Critical
            } else {
                FindingSeverity::Warning
            },
            title: "Latency is unstable".to_string(),
            evidence: format!(
                "Jitter p95 is {jitter_p95:.1} ms while average jitter is {:.1} ms.",
                latency.jitter_ms
            ),
            recommendation: Some(
                "Check background traffic and Wi-Fi interference; compare against Ethernet if possible."
                    .to_string(),
            ),
        });
    }

    let idle_tail_spread = (analysis.idle.p99_ms - analysis.idle.median_ms).max(0.0);
    if idle_tail_spread >= 30.0 {
        findings.push(DiagnosticFinding {
            severity: FindingSeverity::Warning,
            title: "Idle latency has long-tail spikes".to_string(),
            evidence: format!(
                "Idle median is {:.1} ms but p99 reaches {:.1} ms (+{idle_tail_spread:.1} ms).",
                analysis.idle.median_ms, analysis.idle.p99_ms
            ),
            recommendation: Some(
                "Look for periodic background traffic, power-saving Wi-Fi behavior, or a congested wireless channel."
                    .to_string(),
            ),
        });
    }

    if download.mbps < 25.0 {
        findings.push(DiagnosticFinding {
            severity: FindingSeverity::Warning,
            title: "Download capacity is limited".to_string(),
            evidence: format!("Measured download throughput is {:.1} Mbps.", download.mbps),
            recommendation: Some(
                "Compare Ethernet and Wi-Fi, then repeat against another backend or server before blaming the access link."
                    .to_string(),
            ),
        });
    }

    if upload.mbps < 5.0 {
        findings.push(DiagnosticFinding {
            severity: FindingSeverity::Warning,
            title: "Upload capacity may constrain real-time apps".to_string(),
            evidence: format!("Measured upload throughput is {:.1} Mbps.", upload.mbps),
            recommendation: Some(
                "Pause cloud backups or uploads and retest; video calls are usually more sensitive to upstream saturation than downloads."
                    .to_string(),
            ),
        });
    }

    if download.mbps >= upload.mbps * 15.0 && upload.mbps < 25.0 {
        findings.push(DiagnosticFinding {
            severity: FindingSeverity::Info,
            title: "Connection is highly asymmetric".to_string(),
            evidence: format!(
                "Download is {:.1} Mbps while upload is {:.1} Mbps.",
                download.mbps, upload.mbps
            ),
            recommendation: Some(
                "Large downloads may feel fast while backups, streaming, and calls remain upload-limited."
                    .to_string(),
            ),
        });
    }

    let has_problem = findings.iter().any(|finding| {
        matches!(
            finding.severity,
            FindingSeverity::Warning | FindingSeverity::Critical
        )
    });
    if !has_problem {
        findings.insert(
            0,
            DiagnosticFinding {
                severity: FindingSeverity::Info,
                title: "Connection looks healthy under this test".to_string(),
                evidence: format!(
                    "Idle median {:.1} ms, jitter p95 {:.1} ms, download {:.1} Mbps, upload {:.1} Mbps.",
                    latency.idle_ms, jitter_p95, download.mbps, upload.mbps
                ),
                recommendation: None,
            },
        );
    }

    if latency.packet_loss_percent.is_none() && findings.len() < 5 {
        findings.push(DiagnosticFinding {
            severity: FindingSeverity::Info,
            title: "Packet loss is not included yet".to_string(),
            evidence: "The current backend does not expose a reliable packet-loss measurement, so real-time workload grades do not include loss.".to_string(),
            recommendation: None,
        });
    }

    findings
}

fn max_option(left: Option<f64>, right: Option<f64>) -> Option<f64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn bufferbloat_grade(increase_ms: f64) -> QualityGrade {
    if increase_ms <= 5.0 {
        QualityGrade::APlus
    } else if increase_ms <= 15.0 {
        QualityGrade::A
    } else if increase_ms <= 30.0 {
        QualityGrade::B
    } else if increase_ms <= 60.0 {
        QualityGrade::C
    } else if increase_ms <= 120.0 {
        QualityGrade::D
    } else {
        QualityGrade::F
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

fn weighted_score(components: &[(f64, f64)]) -> f64 {
    let total_weight: f64 = components.iter().map(|(_, weight)| weight).sum();
    if total_weight <= f64::EPSILON {
        return 0.0;
    }

    components
        .iter()
        .map(|(score, weight)| score * weight)
        .sum::<f64>()
        / total_weight
}

fn apply_packet_loss_penalty(score: f64, packet_loss_percent: Option<f64>) -> f64 {
    let penalty = match packet_loss_percent {
        Some(loss) if loss >= 5.0 => 35.0,
        Some(loss) if loss >= 2.0 => 25.0,
        Some(loss) if loss >= 1.0 => 15.0,
        Some(loss) if loss >= 0.2 => 7.0,
        _ => 0.0,
    };
    (score - penalty).max(0.0)
}

fn lower_is_better(value: f64, bands: &[(f64, f64)]) -> f64 {
    bands
        .iter()
        .find_map(|(limit, score)| (value <= *limit).then_some(*score))
        .unwrap_or(5.0)
}

fn higher_is_better(value: f64, bands: &[(f64, f64)]) -> f64 {
    bands
        .iter()
        .find_map(|(limit, score)| (value >= *limit).then_some(*score))
        .unwrap_or(15.0)
}

const IDLE_BANDS: [(f64, f64); 6] = [
    (10.0, 100.0),
    (20.0, 95.0),
    (40.0, 85.0),
    (70.0, 70.0),
    (120.0, 50.0),
    (200.0, 30.0),
];
const JITTER_BANDS: [(f64, f64); 6] = [
    (2.0, 100.0),
    (5.0, 95.0),
    (10.0, 85.0),
    (20.0, 65.0),
    (40.0, 40.0),
    (80.0, 20.0),
];
const TAIL_BANDS: [(f64, f64); 6] = [
    (20.0, 100.0),
    (40.0, 95.0),
    (70.0, 82.0),
    (120.0, 65.0),
    (200.0, 40.0),
    (400.0, 20.0),
];
const BUFFERBLOAT_BANDS: [(f64, f64); 6] = [
    (5.0, 100.0),
    (15.0, 95.0),
    (30.0, 82.0),
    (60.0, 65.0),
    (120.0, 40.0),
    (200.0, 20.0),
];
const DOWNLOAD_BANDS: [(f64, f64); 7] = [
    (500.0, 100.0),
    (200.0, 98.0),
    (100.0, 95.0),
    (50.0, 90.0),
    (25.0, 80.0),
    (10.0, 65.0),
    (5.0, 45.0),
];
const UPLOAD_BANDS: [(f64, f64); 7] = [
    (100.0, 100.0),
    (50.0, 98.0),
    (25.0, 95.0),
    (10.0, 88.0),
    (5.0, 75.0),
    (2.0, 55.0),
    (1.0, 40.0),
];
const VIDEO_LATENCY_BANDS: [(f64, f64); 5] = [
    (30.0, 100.0),
    (60.0, 92.0),
    (100.0, 80.0),
    (150.0, 60.0),
    (250.0, 35.0),
];
const VIDEO_JITTER_BANDS: [(f64, f64); 5] = [
    (5.0, 100.0),
    (10.0, 92.0),
    (20.0, 78.0),
    (30.0, 60.0),
    (50.0, 35.0),
];
const VIDEO_UPLOAD_BANDS: [(f64, f64); 5] = [
    (10.0, 100.0),
    (5.0, 95.0),
    (3.0, 85.0),
    (1.5, 65.0),
    (0.8, 40.0),
];
const VIDEO_LOADED_BANDS: [(f64, f64); 5] = [
    (15.0, 100.0),
    (30.0, 90.0),
    (60.0, 75.0),
    (120.0, 50.0),
    (200.0, 25.0),
];
const STREAMING_DOWNLOAD_BANDS: [(f64, f64); 6] = [
    (100.0, 100.0),
    (50.0, 97.0),
    (25.0, 92.0),
    (15.0, 78.0),
    (8.0, 58.0),
    (4.0, 35.0),
];
const STREAMING_JITTER_BANDS: [(f64, f64); 5] = [
    (10.0, 100.0),
    (20.0, 95.0),
    (40.0, 85.0),
    (80.0, 65.0),
    (150.0, 40.0),
];
const STREAMING_LOADED_BANDS: [(f64, f64); 5] = [
    (30.0, 100.0),
    (60.0, 92.0),
    (120.0, 78.0),
    (200.0, 55.0),
    (350.0, 30.0),
];
const CLOUD_LATENCY_BANDS: [(f64, f64); 5] = [
    (20.0, 100.0),
    (35.0, 92.0),
    (60.0, 78.0),
    (100.0, 55.0),
    (160.0, 30.0),
];
const CLOUD_JITTER_BANDS: [(f64, f64); 5] = [
    (3.0, 100.0),
    (7.0, 92.0),
    (15.0, 75.0),
    (30.0, 50.0),
    (60.0, 25.0),
];
const CLOUD_DOWNLOAD_BANDS: [(f64, f64); 5] = [
    (50.0, 100.0),
    (25.0, 95.0),
    (15.0, 85.0),
    (10.0, 70.0),
    (5.0, 40.0),
];
const CLOUD_LOADED_BANDS: [(f64, f64); 5] = [
    (10.0, 100.0),
    (25.0, 90.0),
    (50.0, 75.0),
    (100.0, 50.0),
    (180.0, 25.0),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn throughput(mbps: f64) -> ThroughputResult {
        ThroughputResult {
            mbps,
            bytes: 1,
            seconds: 1.0,
        }
    }

    #[test]
    fn distribution_interpolates_tail_percentiles() {
        let stats = distribution(&[10.0, 20.0, 30.0, 40.0, 50.0]).unwrap();
        assert_eq!(stats.samples, 5);
        assert!((stats.median_ms - 30.0).abs() < 0.001);
        assert!((stats.p95_ms - 48.0).abs() < 0.001);
        assert!((stats.p99_ms - 49.6).abs() < 0.001);
    }

    #[test]
    fn summary_keeps_legacy_medians_and_mean_jitter() {
        let idle = [10.0, 12.0, 11.0, 15.0];
        let latency = summarize_latency(&idle, &[30.0, 32.0], &[40.0, 44.0], None);
        assert_eq!(latency.idle_ms, 11.5);
        assert!((latency.jitter_ms - (7.0 / 3.0)).abs() < 0.001);
        assert_eq!(latency.download_loaded_ms, Some(31.0));
        assert_eq!(latency.upload_loaded_ms, Some(42.0));
    }

    #[test]
    fn severe_upload_bufferbloat_is_explained() {
        let idle: Vec<f64> = (0..24).map(|index| 8.0 + (index % 3) as f64).collect();
        let download_loaded = vec![22.0; 12];
        let upload_loaded = vec![190.0; 12];
        let latency = summarize_latency(&idle, &download_loaded, &upload_loaded, None);
        let analysis = build_network_analysis(
            &idle,
            &download_loaded,
            &upload_loaded,
            &latency,
            &throughput(700.0),
            &throughput(80.0),
        );

        assert_eq!(analysis.quality.bufferbloat.grade, Some(QualityGrade::F));
        assert!(analysis.quality.findings.iter().any(|finding| {
            finding.severity == FindingSeverity::Critical
                && finding.title.contains("upload bufferbloat")
        }));
    }

    #[test]
    fn healthy_connection_scores_high() {
        let idle: Vec<f64> = (0..24).map(|index| 7.0 + (index % 2) as f64).collect();
        let download_loaded = vec![12.0; 12];
        let upload_loaded = vec![14.0; 12];
        let latency = summarize_latency(&idle, &download_loaded, &upload_loaded, None);
        let analysis = build_network_analysis(
            &idle,
            &download_loaded,
            &upload_loaded,
            &latency,
            &throughput(900.0),
            &throughput(200.0),
        );

        assert!(analysis.quality.score >= 90);
        assert!(matches!(
            analysis.quality.grade,
            QualityGrade::A | QualityGrade::APlus
        ));
        assert_eq!(analysis.quality.confidence, QualityConfidence::High);
    }

    #[test]
    fn missing_loaded_latency_reduces_confidence_without_inventing_bufferbloat() {
        let idle = vec![12.0; 24];
        let latency = summarize_latency(&idle, &[], &[], None);
        let analysis = build_network_analysis(
            &idle,
            &[],
            &[],
            &latency,
            &throughput(100.0),
            &throughput(20.0),
        );

        assert_eq!(analysis.quality.confidence, QualityConfidence::Limited);
        assert_eq!(analysis.quality.bufferbloat.grade, None);
    }
}
