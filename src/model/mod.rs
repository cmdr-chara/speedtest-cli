use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestPhase {
    Preparing,
    Latency,
    Download,
    Upload,
    Complete,
}

impl TestPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preparing => "PREPARING",
            Self::Latency => "LATENCY",
            Self::Download => "DOWNLOAD",
            Self::Upload => "UPLOAD",
            Self::Complete => "COMPLETE",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub host: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyResult {
    pub idle_ms: f64,
    pub jitter_ms: f64,
    pub download_loaded_ms: Option<f64>,
    pub upload_loaded_ms: Option<f64>,
    pub packet_loss_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThroughputResult {
    pub mbps: f64,
    pub bytes: u64,
    pub seconds: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LatencyDistribution {
    pub samples: usize,
    pub min_ms: f64,
    pub median_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyAnalysis {
    pub idle: LatencyDistribution,
    pub jitter: Option<LatencyDistribution>,
    pub download_loaded: Option<LatencyDistribution>,
    pub upload_loaded: Option<LatencyDistribution>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityGrade {
    APlus,
    A,
    B,
    C,
    D,
    F,
}

impl QualityGrade {
    pub fn label(self) -> &'static str {
        match self {
            Self::APlus => "A+",
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
            Self::F => "F",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityConfidence {
    High,
    Moderate,
    Limited,
}

impl QualityConfidence {
    pub fn label(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Moderate => "moderate",
            Self::Limited => "limited",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Info,
    Warning,
    Critical,
}

impl FindingSeverity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warning => "WARNING",
            Self::Critical => "CRITICAL",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferbloatAssessment {
    pub download_increase_ms: Option<f64>,
    pub upload_increase_ms: Option<f64>,
    pub worst_increase_ms: Option<f64>,
    pub grade: Option<QualityGrade>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadGrades {
    pub gaming: QualityGrade,
    pub video_calls: QualityGrade,
    pub streaming: QualityGrade,
    pub cloud_gaming: QualityGrade,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticFinding {
    pub severity: FindingSeverity,
    pub title: String,
    pub evidence: String,
    pub recommendation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkQuality {
    pub score: u8,
    pub grade: QualityGrade,
    pub confidence: QualityConfidence,
    pub bufferbloat: BufferbloatAssessment,
    pub workloads: WorkloadGrades,
    pub findings: Vec<DiagnosticFinding>,
}

impl NetworkQuality {
    pub const S_TIER_SCORE: u8 = 98;

    pub fn is_s_tier(&self) -> bool {
        self.score >= Self::S_TIER_SCORE
            && self.grade == QualityGrade::APlus
            && self.confidence == QualityConfidence::High
    }

    pub fn tier_label(&self) -> Option<&'static str> {
        self.is_s_tier().then_some("S-TIER")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkAnalysis {
    pub latency: LatencyAnalysis,
    pub quality: NetworkQuality,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub timestamp: DateTime<Utc>,
    pub backend: String,
    pub server: ServerInfo,
    pub latency: LatencyResult,
    pub download: ThroughputResult,
    pub upload: ThroughputResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<Box<NetworkAnalysis>>,
}

impl TestResult {
    pub fn pretty_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Validate values at persistence and automation boundaries. Serde checks
    /// shape, but it cannot express the physical invariants of a measurement;
    /// without this guard a hand-edited result could enter history and skew
    /// every offline statistic.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.backend.trim().is_empty(), "result backend is empty");
        anyhow::ensure!(
            !self.server.host.trim().is_empty(),
            "result server host is empty"
        );
        anyhow::ensure!(
            !self.server.name.trim().is_empty(),
            "result server name is empty"
        );
        validate_nonnegative("idle latency", self.latency.idle_ms)?;
        validate_nonnegative("jitter", self.latency.jitter_ms)?;
        for (name, value) in [
            ("download loaded latency", self.latency.download_loaded_ms),
            ("upload loaded latency", self.latency.upload_loaded_ms),
        ] {
            if let Some(value) = value {
                validate_nonnegative(name, value)?;
            }
        }
        if let Some(loss) = self.latency.packet_loss_percent {
            anyhow::ensure!(
                loss.is_finite() && (0.0..=100.0).contains(&loss),
                "packet loss must be between 0 and 100 percent"
            );
        }
        validate_throughput("download", &self.download)?;
        validate_throughput("upload", &self.upload)?;
        if let Some(analysis) = &self.analysis {
            validate_distribution("idle latency distribution", &analysis.latency.idle)?;
            for (name, distribution) in [
                ("jitter distribution", analysis.latency.jitter.as_ref()),
                (
                    "download loaded latency distribution",
                    analysis.latency.download_loaded.as_ref(),
                ),
                (
                    "upload loaded latency distribution",
                    analysis.latency.upload_loaded.as_ref(),
                ),
            ] {
                if let Some(distribution) = distribution {
                    validate_distribution(name, distribution)?;
                }
            }
            anyhow::ensure!(
                analysis.quality.score <= 100,
                "quality score must be between 0 and 100"
            );
        }
        Ok(())
    }
}

fn validate_nonnegative(name: &str, value: f64) -> anyhow::Result<()> {
    anyhow::ensure!(
        value.is_finite() && value >= 0.0,
        "{name} must be finite and non-negative"
    );
    Ok(())
}

fn validate_throughput(name: &str, value: &ThroughputResult) -> anyhow::Result<()> {
    validate_nonnegative(&format!("{name} throughput"), value.mbps)?;
    anyhow::ensure!(
        value.seconds.is_finite() && value.seconds > 0.0,
        "{name} duration must be finite and positive"
    );
    Ok(())
}

fn validate_distribution(name: &str, value: &LatencyDistribution) -> anyhow::Result<()> {
    anyhow::ensure!(value.samples > 0, "{name} has no samples");
    for (label, number) in [
        ("min", value.min_ms),
        ("median", value.median_ms),
        ("p95", value.p95_ms),
        ("p99", value.p99_ms),
        ("max", value.max_ms),
    ] {
        validate_nonnegative(&format!("{name} {label}"), number)?;
    }
    anyhow::ensure!(
        value.min_ms <= value.median_ms
            && value.median_ms <= value.p95_ms
            && value.p95_ms <= value.p99_ms
            && value.p99_ms <= value.max_ms,
        "{name} percentiles are not ordered"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quality(score: u8, confidence: QualityConfidence) -> NetworkQuality {
        NetworkQuality {
            score,
            grade: QualityGrade::APlus,
            confidence,
            bufferbloat: BufferbloatAssessment {
                download_increase_ms: Some(1.0),
                upload_increase_ms: Some(1.0),
                worst_increase_ms: Some(1.0),
                grade: Some(QualityGrade::APlus),
            },
            workloads: WorkloadGrades {
                gaming: QualityGrade::APlus,
                video_calls: QualityGrade::APlus,
                streaming: QualityGrade::APlus,
                cloud_gaming: QualityGrade::APlus,
            },
            findings: Vec::new(),
        }
    }

    #[test]
    fn s_tier_requires_exceptional_score_and_high_confidence() {
        assert!(quality(98, QualityConfidence::High).is_s_tier());
        assert!(quality(100, QualityConfidence::High).is_s_tier());
        assert!(!quality(97, QualityConfidence::High).is_s_tier());
        assert!(!quality(100, QualityConfidence::Moderate).is_s_tier());
        assert_eq!(
            quality(99, QualityConfidence::High).tier_label(),
            Some("S-TIER")
        );
    }

    #[test]
    fn validates_physical_measurement_invariants() {
        let mut result: TestResult =
            serde_json::from_str(include_str!("../../tests/fixtures/result.json")).unwrap();
        result.validate().unwrap();
        result.download.mbps = -1.0;
        assert!(result.validate().is_err());
        result.download.mbps = 100.0;
        result.download.seconds = 0.0;
        assert!(result.validate().is_err());
        result.download.seconds = 1.0;
        result.latency.packet_loss_percent = Some(101.0);
        assert!(result.validate().is_err());
    }
}
