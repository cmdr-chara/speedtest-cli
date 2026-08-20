use std::{net::IpAddr, process::Command, time::Duration};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpStream, time::timeout};

use crate::{
    dns,
    model::{FindingSeverity, TestResult},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorStatus {
    Pass,
    Warning,
    Fail,
    NotAvailable,
}

impl DoctorStatus {
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Pass => "✓",
            Self::Warning => "⚠",
            Self::Fail => "✗",
            Self::NotAvailable => "·",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: DoctorStatus,
    pub detail: String,
    pub metric_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    pub timestamp: DateTime<Utc>,
    pub interface: Option<String>,
    pub gateway: Option<IpAddr>,
    pub checks: Vec<DoctorCheck>,
    pub diagnosis: String,
    pub recommendation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speedtest: Option<TestResult>,
}

impl DoctorReport {
    pub fn attach_speedtest(&mut self, result: TestResult) {
        self.speedtest = Some(result);
        self.refresh_diagnosis();
    }

    pub fn pretty_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    fn refresh_diagnosis(&mut self) {
        if let Some(result) = &self.speedtest {
            if let Some(analysis) = &result.analysis {
                if let Some(finding) = analysis.quality.findings.iter().find(|finding| {
                    matches!(
                        finding.severity,
                        FindingSeverity::Warning | FindingSeverity::Critical
                    )
                }) {
                    self.diagnosis = finding.title.clone();
                    self.recommendation = finding.recommendation.clone();
                    return;
                }
            }
        }

        if let Some(check) = self
            .checks
            .iter()
            .find(|check| check.name == "DNS resolver" && check.status != DoctorStatus::Pass)
        {
            self.diagnosis =
                "DNS resolution is the clearest problem in this diagnostic pass".to_string();
            self.recommendation = Some(format!(
                "{} Run `speedtest dns benchmark` to compare alternative resolvers before changing configuration.",
                check.detail
            ));
            return;
        }

        if self
            .checks
            .iter()
            .any(|check| check.name == "IPv4 internet" && check.status == DoctorStatus::Fail)
        {
            self.diagnosis = "IPv4 internet connectivity failed".to_string();
            self.recommendation = Some(
                "Check the default gateway, VPN/firewall state, and upstream connection before focusing on throughput."
                    .to_string(),
            );
            return;
        }

        if let Some(check) = self.checks.iter().find(|check| {
            check.name == "Gateway latency"
                && matches!(check.status, DoctorStatus::Warning | DoctorStatus::Fail)
        }) {
            self.diagnosis =
                "Latency is already elevated on the local path to the gateway".to_string();
            self.recommendation = Some(format!(
                "{} Compare Ethernet and Wi-Fi and inspect local wireless contention before blaming the ISP.",
                check.detail
            ));
            return;
        }

        self.diagnosis = "No clear local connectivity fault was detected".to_string();
        self.recommendation = Some(
            "Use `speedtest doctor --full` for throughput/bufferbloat analysis or `speedtest stability` for intermittent issues."
                .to_string(),
        );
    }
}

pub async fn run(interface: Option<&str>) -> Result<DoctorReport> {
    let state = dns::system::inspect(interface)?;
    let mut checks = Vec::new();

    checks.push(DoctorCheck {
        name: "Default route".to_string(),
        status: DoctorStatus::Pass,
        detail: format!("active interface: {}", state.interface),
        metric_ms: None,
    });

    checks.push(DoctorCheck {
        name: "DNS configuration".to_string(),
        status: if state.servers.is_empty() {
            DoctorStatus::Warning
        } else {
            DoctorStatus::Pass
        },
        detail: if state.servers.is_empty() {
            format!("{} did not expose effective DNS servers", state.backend)
        } else {
            format!(
                "{} via {} ({})",
                state
                    .servers
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                state.backend,
                state.mode.label()
            )
        },
        metric_ms: None,
    });

    if let Some(gateway) = state.gateway {
        let ping = tokio::task::spawn_blocking(move || gateway_ping(gateway))
            .await
            .context("gateway ping task panicked")?;
        match ping {
            Ok(Some(ms)) => checks.push(DoctorCheck {
                name: "Gateway latency".to_string(),
                status: if ms <= 5.0 {
                    DoctorStatus::Pass
                } else if ms <= 20.0 {
                    DoctorStatus::Warning
                } else {
                    DoctorStatus::Fail
                },
                detail: format!("{gateway} responded in {ms:.1} ms"),
                metric_ms: Some(ms),
            }),
            Ok(None) => checks.push(DoctorCheck {
                name: "Gateway latency".to_string(),
                status: DoctorStatus::NotAvailable,
                detail: format!("{gateway} did not expose a parseable ping time"),
                metric_ms: None,
            }),
            Err(error) => checks.push(DoctorCheck {
                name: "Gateway latency".to_string(),
                status: DoctorStatus::NotAvailable,
                detail: format!("gateway may block ICMP: {error}"),
                metric_ms: None,
            }),
        }
    } else {
        checks.push(DoctorCheck {
            name: "Gateway latency".to_string(),
            status: DoctorStatus::NotAvailable,
            detail: "default gateway address was not exposed by the platform".to_string(),
            metric_ms: None,
        });
    }

    let ipv4 = tcp_check("1.1.1.1:443").await;
    checks.push(connectivity_check("IPv4 internet", ipv4));

    if state.ipv6_default_route {
        let ipv6 = tcp_check("[2606:4700:4700::1111]:443").await;
        checks.push(connectivity_check("IPv6 internet", ipv6));
    } else {
        checks.push(DoctorCheck {
            name: "IPv6 internet".to_string(),
            status: DoctorStatus::NotAvailable,
            detail: "no IPv6 default route detected".to_string(),
            metric_ms: None,
        });
    }

    match dns::test_current(6).await {
        Ok(result) => {
            let median = result.latency.as_ref().map(|latency| latency.median_ms);
            let status = if result.success_rate_percent < 80.0 {
                DoctorStatus::Fail
            } else if result.success_rate_percent < 100.0
                || median.is_some_and(|median| median > 50.0)
            {
                DoctorStatus::Warning
            } else {
                DoctorStatus::Pass
            };
            checks.push(DoctorCheck {
                name: "DNS resolver".to_string(),
                status,
                detail: median.map_or_else(
                    || format!("{:.0}% query success", result.success_rate_percent),
                    |median| {
                        format!(
                            "{median:.1} ms median, {:.0}% query success",
                            result.success_rate_percent
                        )
                    },
                ),
                metric_ms: median,
            });
        }
        Err(error) => checks.push(DoctorCheck {
            name: "DNS resolver".to_string(),
            status: DoctorStatus::Fail,
            detail: format!("DNS test failed: {error:#}"),
            metric_ms: None,
        }),
    }

    let https_started = tokio::time::Instant::now();
    let https = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build doctor HTTP client")?
        .get("https://example.com/")
        .send()
        .await;
    checks.push(match https {
        Ok(response) if response.status().is_success() => DoctorCheck {
            name: "HTTPS".to_string(),
            status: DoctorStatus::Pass,
            detail: format!(
                "TLS/HTTPS request succeeded in {:.1} ms",
                https_started.elapsed().as_secs_f64() * 1000.0
            ),
            metric_ms: Some(https_started.elapsed().as_secs_f64() * 1000.0),
        },
        Ok(response) => DoctorCheck {
            name: "HTTPS".to_string(),
            status: DoctorStatus::Warning,
            detail: format!("HTTPS endpoint returned {}", response.status()),
            metric_ms: None,
        },
        Err(error) => DoctorCheck {
            name: "HTTPS".to_string(),
            status: DoctorStatus::Fail,
            detail: format!("HTTPS request failed: {error}"),
            metric_ms: None,
        },
    });

    let mut report = DoctorReport {
        timestamp: Utc::now(),
        interface: Some(state.interface),
        gateway: state.gateway,
        checks,
        diagnosis: String::new(),
        recommendation: None,
        speedtest: None,
    };
    report.refresh_diagnosis();
    Ok(report)
}

async fn tcp_check(address: &str) -> Result<f64> {
    let started = tokio::time::Instant::now();
    timeout(Duration::from_secs(3), TcpStream::connect(address))
        .await
        .context("connection timed out")??;
    Ok(started.elapsed().as_secs_f64() * 1000.0)
}

fn connectivity_check(name: &str, result: Result<f64>) -> DoctorCheck {
    match result {
        Ok(ms) => DoctorCheck {
            name: name.to_string(),
            status: DoctorStatus::Pass,
            detail: format!("TCP/443 reachable in {ms:.1} ms"),
            metric_ms: Some(ms),
        },
        Err(error) => DoctorCheck {
            name: name.to_string(),
            status: DoctorStatus::Fail,
            detail: format!("TCP/443 connectivity failed: {error:#}"),
            metric_ms: None,
        },
    }
}

fn gateway_ping(gateway: IpAddr) -> Result<Option<f64>> {
    #[cfg(target_os = "windows")]
    let output = Command::new("ping")
        .args(["-n", "1", "-w", "1000", &gateway.to_string()])
        .output()
        .context("failed to run ping")?;

    #[cfg(target_os = "macos")]
    let output = Command::new("ping")
        .args(["-c", "1", "-W", "1000", &gateway.to_string()])
        .output()
        .context("failed to run ping")?;

    #[cfg(all(unix, not(target_os = "macos")))]
    let output = Command::new("ping")
        .args(["-c", "1", "-W", "1", &gateway.to_string()])
        .output()
        .context("failed to run ping")?;

    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    return Ok(None);

    if !output.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse_ping_time(&text))
}

fn parse_ping_time(text: &str) -> Option<f64> {
    for marker in ["time=", "time<"] {
        if let Some((_, tail)) = text.split_once(marker) {
            let value = tail
                .chars()
                .take_while(|character| character.is_ascii_digit() || *character == '.')
                .collect::<String>();
            if let Ok(ms) = value.parse::<f64>() {
                return Some(ms);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_ping_formats() {
        assert_eq!(parse_ping_time("64 bytes time=1.42 ms"), Some(1.42));
        assert_eq!(parse_ping_time("time<1ms TTL=64"), Some(1.0));
    }
}
