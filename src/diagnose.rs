//! Guided connection-health assessment built from the existing diagnostic
//! primitives. The orchestration is deliberately separate from terminal
//! rendering so JSON reports and future cockpit screens use the same result.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    doctor::{DoctorReport, DoctorStatus},
    model::{FindingSeverity, TestResult},
    network::AdvancedNetworkReport,
    stability::StabilityResult,
};

pub const SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisProfile {
    General,
    Calls,
    Gaming,
    Streaming,
}

impl DiagnosisProfile {
    pub const fn label(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Calls => "calls",
            Self::Gaming => "gaming",
            Self::Streaming => "streaming",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosisFinding {
    pub severity: FindingSeverity,
    pub area: String,
    pub title: String,
    pub evidence: String,
    pub recommendation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosisReport {
    pub schema_version: u8,
    pub timestamp: DateTime<Utc>,
    pub profile: DiagnosisProfile,
    pub diagnosis: String,
    pub confidence: String,
    pub recommendations: Vec<String>,
    pub findings: Vec<DiagnosisFinding>,
    pub doctor: DoctorReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advanced: Option<AdvancedNetworkReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stability: Option<StabilityResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speedtest: Option<TestResult>,
}

impl DiagnosisReport {
    pub fn pretty_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

pub fn build(
    profile: DiagnosisProfile,
    doctor: DoctorReport,
    stability: Option<StabilityResult>,
    speedtest: Option<TestResult>,
    advanced: Option<AdvancedNetworkReport>,
) -> DiagnosisReport {
    let mut findings = Vec::new();
    let mut recommendations = Vec::new();

    for check in &doctor.checks {
        let (severity, area) = match check.status {
            DoctorStatus::Pass => continue,
            DoctorStatus::Warning => (FindingSeverity::Warning, "connectivity"),
            DoctorStatus::Fail => (FindingSeverity::Critical, "connectivity"),
            DoctorStatus::NotAvailable => (FindingSeverity::Info, "availability"),
        };
        push_finding(
            &mut findings,
            DiagnosisFinding {
                severity,
                area: area.to_string(),
                title: check.name.clone(),
                evidence: check.detail.clone(),
                recommendation: None,
            },
        );
    }

    if doctor.diagnosis != "No clear local connectivity fault was detected"
        && doctor.diagnosis
            != "No clear local fault was detected, but the full throughput test was unavailable"
    {
        recommendations.push(doctor.diagnosis.clone());
    }
    if let Some(recommendation) = &doctor.recommendation {
        push_recommendation(&mut recommendations, recommendation.clone());
    }

    if let Some(result) = &speedtest {
        if let Some(analysis) = &result.analysis {
            for finding in &analysis.quality.findings {
                push_finding(
                    &mut findings,
                    DiagnosisFinding {
                        severity: finding.severity,
                        area: "quality".to_string(),
                        title: finding.title.clone(),
                        evidence: finding.evidence.clone(),
                        recommendation: finding.recommendation.clone(),
                    },
                );
                if let Some(recommendation) = &finding.recommendation {
                    push_recommendation(&mut recommendations, recommendation.clone());
                }
            }
        }
        add_profile_finding(profile, result, &mut findings, &mut recommendations);
    }

    if let Some(stability) = &stability {
        if stability.probe_availability_percent < 98.0 {
            let severity = if stability.probe_availability_percent < 90.0 {
                FindingSeverity::Critical
            } else {
                FindingSeverity::Warning
            };
            push_finding(
                &mut findings,
                DiagnosisFinding {
                    severity,
                    area: "stability".to_string(),
                    title: "Intermittent availability".to_string(),
                    evidence: format!(
                        "HTTP probe availability was {:.1}% with {} failure burst(s)",
                        stability.probe_availability_percent, stability.failure_bursts
                    ),
                    recommendation: Some(
                        "Repeat the assessment at a different time and compare Ethernet, Wi-Fi, and VPN paths.".to_string(),
                    ),
                },
            );
            push_recommendation(
                &mut recommendations,
                "Repeat the assessment at a different time and compare Ethernet, Wi-Fi, and VPN paths.".to_string(),
            );
        }
    }

    if let Some(advanced) = &advanced {
        if advanced.vpn.likely {
            push_finding(
                &mut findings,
                DiagnosisFinding {
                    severity: FindingSeverity::Info,
                    area: "path".to_string(),
                    title: "A tunnel or VPN interface is active".to_string(),
                    evidence: advanced.vpn.signals.join(", "),
                    recommendation: Some(
                        "Repeat the same profile with the VPN disabled when comparing ISP or Wi-Fi performance.".to_string(),
                    ),
                },
            );
        }
        if let Some(mtu) = &advanced.mtu {
            if mtu.path_mtu_bytes < 1400 {
                push_finding(
                    &mut findings,
                    DiagnosisFinding {
                        severity: FindingSeverity::Warning,
                        area: "path".to_string(),
                        title: "Reduced path MTU".to_string(),
                        evidence: format!("discovered path MTU is {} bytes", mtu.path_mtu_bytes),
                        recommendation: Some(
                            "Check VPN encapsulation, tunnel overhead, and router MTU settings."
                                .to_string(),
                        ),
                    },
                );
            }
        }
    }

    findings.sort_by_key(|finding| match finding.severity {
        FindingSeverity::Critical => 0_u8,
        FindingSeverity::Warning => 1,
        FindingSeverity::Info => 2,
    });
    let diagnosis = findings
        .iter()
        .find(|finding| !matches!(finding.severity, FindingSeverity::Info))
        .map(|finding| finding.title.clone())
        .unwrap_or_else(|| doctor.diagnosis.clone());
    let evidence_count = usize::from(speedtest.is_some())
        + usize::from(stability.is_some())
        + usize::from(!doctor.checks.is_empty());
    let confidence = match evidence_count {
        0 | 1 => "limited",
        2 => "moderate",
        _ => "high",
    }
    .to_string();

    DiagnosisReport {
        schema_version: SCHEMA_VERSION,
        timestamp: Utc::now(),
        profile,
        diagnosis,
        confidence,
        recommendations,
        findings,
        doctor,
        advanced,
        stability,
        speedtest,
    }
}

fn add_profile_finding(
    profile: DiagnosisProfile,
    result: &TestResult,
    findings: &mut Vec<DiagnosisFinding>,
    recommendations: &mut Vec<String>,
) {
    let loaded_values: Vec<f64> = [
        result.latency.download_loaded_ms,
        result.latency.upload_loaded_ms,
    ]
    .into_iter()
    .flatten()
    .collect();
    let loaded = loaded_values.iter().copied().fold(0.0, f64::max);
    let jitter = result.latency.jitter_ms;
    let packet_loss = result.latency.packet_loss_percent;
    if matches!(profile, DiagnosisProfile::Calls | DiagnosisProfile::Gaming)
        && (loaded_values.is_empty() || packet_loss.is_none())
    {
        let mut missing = Vec::new();
        if loaded_values.is_empty() {
            missing.push("loaded latency");
        }
        if packet_loss.is_none() {
            missing.push("packet loss");
        }
        let severity = if loaded_values.is_empty() {
            FindingSeverity::Warning
        } else {
            FindingSeverity::Info
        };
        let title = "Real-time profile evidence is incomplete";
        let recommendation = "Repeat with a backend that provides loaded-latency coverage and run `speedtest loss` separately when packet-loss evidence matters.";
        push_finding(
            findings,
            DiagnosisFinding {
                severity,
                area: "profile".to_string(),
                title: title.to_string(),
                evidence: format!("Unavailable evidence: {}.", missing.join(", ")),
                recommendation: Some(recommendation.to_string()),
            },
        );
        push_recommendation(recommendations, recommendation.to_string());
    }
    let (title, evidence, recommendation) = match profile {
        DiagnosisProfile::Calls
            if loaded > 100.0
                || jitter > 30.0
                || packet_loss.is_some_and(|loss| loss > 1.0) =>
        (
            "Real-time calls may be affected",
            format!("loaded latency {loaded:.1} ms, jitter {jitter:.1} ms, packet loss {}", format_optional_percent(packet_loss)),
            "Check competing uploads, Wi-Fi airtime, and bufferbloat before changing the video-call application.",
        ),
        DiagnosisProfile::Gaming
            if loaded > 60.0
                || jitter > 20.0
                || packet_loss.is_some_and(|loss| loss > 0.5) =>
        (
            "Interactive gaming may be affected",
            format!("loaded latency {loaded:.1} ms, jitter {jitter:.1} ms, packet loss {}", format_optional_percent(packet_loss)),
            "Compare a wired path and enable SQM/CAKE if the router supports it.",
        ),
        DiagnosisProfile::Streaming if result.download.mbps < 25.0 => (
            "Streaming capacity is limited",
            format!("measured download capacity is {:.1} Mbps", result.download.mbps),
            "Repeat against another backend and check whether another device is consuming capacity.",
        ),
        _ => return,
    };
    let finding = DiagnosisFinding {
        severity: FindingSeverity::Warning,
        area: "profile".to_string(),
        title: title.to_string(),
        evidence,
        recommendation: Some(recommendation.to_string()),
    };
    push_finding(findings, finding);
    push_recommendation(recommendations, recommendation.to_string());
}

fn format_optional_percent(value: Option<f64>) -> String {
    value.map_or_else(|| "unavailable".to_string(), |value| format!("{value:.1}%"))
}

fn push_finding(findings: &mut Vec<DiagnosisFinding>, finding: DiagnosisFinding) {
    if !findings
        .iter()
        .any(|existing| existing.title == finding.title && existing.area == finding.area)
    {
        findings.push(finding);
    }
}

fn push_recommendation(recommendations: &mut Vec<String>, recommendation: String) {
    if !recommendations
        .iter()
        .any(|existing| existing == &recommendation)
    {
        recommendations.push(recommendation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        doctor::DoctorCheck,
        model::{LatencyResult, ServerInfo, ThroughputResult},
    };

    fn doctor() -> DoctorReport {
        DoctorReport {
            timestamp: Utc::now(),
            interface: Some("fixture0".to_string()),
            gateway: None,
            checks: vec![DoctorCheck {
                name: "IPv4 internet".to_string(),
                status: DoctorStatus::Pass,
                detail: "reachable".to_string(),
                metric_ms: Some(4.0),
            }],
            diagnosis: "No clear local connectivity fault was detected".to_string(),
            recommendation: None,
            speedtest: None,
        }
    }

    fn speedtest() -> TestResult {
        TestResult {
            timestamp: Utc::now(),
            backend: "fixture".to_string(),
            server: ServerInfo {
                host: "fixture.test".to_string(),
                name: "Fixture".to_string(),
            },
            latency: LatencyResult {
                idle_ms: 10.0,
                jitter_ms: 35.0,
                download_loaded_ms: Some(120.0),
                upload_loaded_ms: Some(110.0),
                packet_loss_percent: Some(0.0),
            },
            download: ThroughputResult {
                mbps: 100.0,
                bytes: 1_000,
                seconds: 1.0,
            },
            upload: ThroughputResult {
                mbps: 50.0,
                bytes: 500,
                seconds: 1.0,
            },
            analysis: None,
        }
    }

    #[test]
    fn profile_adds_workload_specific_evidence() {
        let report = build(
            DiagnosisProfile::Calls,
            doctor(),
            None,
            Some(speedtest()),
            None,
        );
        assert_eq!(report.schema_version, SCHEMA_VERSION);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.title == "Real-time calls may be affected"));
        assert_eq!(report.confidence, "moderate");
    }

    #[test]
    fn missing_realtime_evidence_is_reported_instead_of_looking_healthy() {
        let mut result = speedtest();
        result.latency.download_loaded_ms = None;
        result.latency.upload_loaded_ms = None;
        result.latency.packet_loss_percent = None;
        let report = build(DiagnosisProfile::Gaming, doctor(), None, Some(result), None);
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.title == "Real-time profile evidence is incomplete")
            .unwrap();
        assert_eq!(finding.severity, FindingSeverity::Warning);
        assert!(finding.evidence.contains("loaded latency"));
        assert!(finding.evidence.contains("packet loss"));
    }
}
