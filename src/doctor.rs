use std::{net::IpAddr, process::Command, time::Duration};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpStream, time::timeout};

use crate::{
    dns, dns_custom,
    model::{FindingSeverity, TestResult},
    wifi,
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

    pub fn attach_speedtest_failure(&mut self, error: &anyhow::Error) {
        self.checks.push(DoctorCheck {
            name: "Full speed test".to_string(),
            status: DoctorStatus::NotAvailable,
            detail: format!("throughput/bufferbloat measurement could not complete: {error:#}"),
            metric_ms: None,
        });
        self.refresh_diagnosis();
    }

    pub fn pretty_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    fn refresh_diagnosis(&mut self) {
        if let Some(check) = self
            .checks
            .iter()
            .find(|check| check.name == "Default route" && check.status == DoctorStatus::Fail)
        {
            self.diagnosis = "The selected interface has no default Internet route".to_string();
            self.recommendation = Some(format!(
                "{} Select an interface with an active default route or repair its route configuration.",
                check.detail
            ));
            return;
        }

        if let Some(check) = self.checks.iter().find(|check| {
            check.name == "Probe routing" && check.status == DoctorStatus::NotAvailable
        }) {
            self.diagnosis =
                "Active probes cannot be attributed to the selected interface".to_string();
            self.recommendation = Some(format!(
                "{} Run Doctor on the active default interface for routed DNS, HTTPS, and Internet checks.",
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

        if self
            .checks
            .iter()
            .any(|check| check.name == "IPv6 internet" && check.status == DoctorStatus::Fail)
        {
            self.diagnosis = "IPv6 internet connectivity failed".to_string();
            self.recommendation = Some(
                "Check the IPv6 default route, router advertisement, VPN/firewall state, and upstream IPv6 service."
                    .to_string(),
            );
            return;
        }

        if let Some(check) = self
            .checks
            .iter()
            .find(|check| check.name == "HTTPS" && check.status == DoctorStatus::Fail)
        {
            self.diagnosis = "HTTPS connectivity failed".to_string();
            self.recommendation = Some(format!(
                "{} Check captive-portal, proxy, VPN, firewall, and TLS interception settings.",
                check.detail
            ));
            return;
        }

        if let Some(check) = self.checks.iter().find(|check| {
            check.name == "DNS resolver"
                && matches!(check.status, DoctorStatus::Warning | DoctorStatus::Fail)
        }) {
            self.diagnosis =
                "DNS resolution is the clearest problem in this diagnostic pass".to_string();
            self.recommendation = Some(format!(
                "{} Run `speedtest dns benchmark` to compare alternative resolvers before changing configuration.",
                check.detail
            ));
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

        if let Some(check) = self.checks.iter().find(|check| {
            check.name == "Wi-Fi"
                && matches!(check.status, DoctorStatus::Warning | DoctorStatus::Fail)
        }) {
            self.diagnosis = "The Wi-Fi link is the clearest local weakness".to_string();
            self.recommendation = Some(format!(
                "{} Move closer to the access point, reduce interference, or compare Ethernet.",
                check.detail
            ));
            return;
        }

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

            self.diagnosis =
                "No clear local connectivity or throughput fault was detected".to_string();
            self.recommendation = Some(
                "Use `speedtest stability` if the problem is intermittent or time-dependent."
                    .to_string(),
            );
            return;
        }

        if let Some(check) = self
            .checks
            .iter()
            .find(|check| check.name == "Full speed test")
        {
            self.diagnosis =
                "No clear local fault was detected, but the full throughput test was unavailable"
                    .to_string();
            self.recommendation = Some(format!(
                "{} Retry `speedtest doctor --full` when the measurement endpoint is reachable.",
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
    let active_interface = if interface.is_some() {
        dns::system::inspect(None)
            .ok()
            .map(|active| active.interface)
    } else {
        Some(state.interface.clone())
    };
    let selected_is_active = active_interface.as_deref() == Some(state.interface.as_str());

    let has_default_route = state.ipv4_default_route || state.ipv6_default_route;
    checks.push(DoctorCheck {
        name: "Default route".to_string(),
        status: if has_default_route {
            DoctorStatus::Pass
        } else {
            DoctorStatus::Fail
        },
        detail: if has_default_route {
            format!("selected interface: {}", state.interface)
        } else {
            format!("{} has no IPv4 or IPv6 default route", state.interface)
        },
        metric_ms: None,
    });

    if interface.is_some() {
        checks.push(DoctorCheck {
            name: "Probe routing".to_string(),
            status: if selected_is_active {
                DoctorStatus::Pass
            } else {
                DoctorStatus::NotAvailable
            },
            detail: if selected_is_active {
                format!("active probes use selected interface {}", state.interface)
            } else {
                format!(
                    "active probes were skipped because {} is not the active default interface{}",
                    state.interface,
                    active_interface
                        .as_deref()
                        .map_or(String::new(), |active| format!(" ({active})"))
                )
            },
            metric_ms: None,
        });
    }

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

    if !selected_is_active {
        checks.push(DoctorCheck {
            name: "Gateway latency".to_string(),
            status: DoctorStatus::NotAvailable,
            detail:
                "gateway probe skipped because it cannot be bound safely to the selected interface"
                    .to_string(),
            metric_ms: None,
        });
    } else if let Some(gateway) = state.gateway {
        let gateway_scope = state.gateway_scope.clone();
        let gateway_label = scoped_gateway_target(gateway, gateway_scope.as_deref());
        let ping =
            tokio::task::spawn_blocking(move || gateway_ping(gateway, gateway_scope.as_deref()))
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
                detail: format!("{gateway_label} responded in {ms:.1} ms"),
                metric_ms: Some(ms),
            }),
            Ok(None) => checks.push(DoctorCheck {
                name: "Gateway latency".to_string(),
                status: DoctorStatus::NotAvailable,
                detail: format!("{gateway_label} did not expose a parseable ping time"),
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

    if state.ipv4_default_route && selected_is_active {
        let ipv4 = tcp_check("1.1.1.1:443").await;
        checks.push(connectivity_check("IPv4 internet", ipv4));
    } else {
        checks.push(DoctorCheck {
            name: "IPv4 internet".to_string(),
            status: DoctorStatus::NotAvailable,
            detail: if selected_is_active {
                "no IPv4 default route detected for the selected interface".to_string()
            } else {
                "probe skipped because the selected interface is not active".to_string()
            },
            metric_ms: None,
        });
    }

    if state.ipv6_default_route && selected_is_active {
        let ipv6 = tcp_check("[2606:4700:4700::1111]:443").await;
        checks.push(connectivity_check("IPv6 internet", ipv6));
    } else {
        checks.push(DoctorCheck {
            name: "IPv6 internet".to_string(),
            status: DoctorStatus::NotAvailable,
            detail: if selected_is_active {
                "no IPv6 default route detected".to_string()
            } else {
                "probe skipped because the selected interface is not active".to_string()
            },
            metric_ms: None,
        });
    }

    let dns_result = if !selected_is_active {
        None
    } else if state.servers.is_empty() {
        Some(Err(anyhow::anyhow!(
            "the selected interface exposed no DNS resolver addresses"
        )))
    } else {
        Some(dns_custom::test_servers(state.servers.clone(), 6).await)
    };
    match dns_result {
        None => checks.push(DoctorCheck {
            name: "DNS resolver".to_string(),
            status: DoctorStatus::NotAvailable,
            detail: "DNS probe skipped because it cannot be bound safely to the selected interface"
                .to_string(),
            metric_ms: None,
        }),
        Some(Err(error)) => checks.push(DoctorCheck {
            name: "DNS resolver".to_string(),
            status: DoctorStatus::Fail,
            detail: format!("DNS test failed: {error:#}"),
            metric_ms: None,
        }),
        Some(Ok(result)) => {
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
    }

    #[cfg(target_os = "macos")]
    let selected_wifi_interface = state.device.as_deref();
    #[cfg(not(target_os = "macos"))]
    let selected_wifi_interface = Some(state.interface.as_str());

    checks.push(match wifi::inspect(selected_wifi_interface) {
        Ok(snapshot) if snapshot.available => {
            let signal_known = snapshot.signal_dbm.is_some() || snapshot.signal_percent.is_some();
            let weak = snapshot.signal_dbm.is_some_and(|dbm| dbm <= -80.0)
                || snapshot
                    .signal_percent
                    .is_some_and(|quality| quality <= 25.0);
            let signal = snapshot
                .signal_dbm
                .map(|dbm| format!("{dbm:.0} dBm"))
                .or_else(|| {
                    snapshot
                        .signal_percent
                        .map(|quality| format!("{quality:.0}%"))
                })
                .unwrap_or_else(|| "signal unavailable".to_string());
            DoctorCheck {
                name: "Wi-Fi".to_string(),
                status: if !signal_known {
                    DoctorStatus::NotAvailable
                } else if weak {
                    DoctorStatus::Warning
                } else {
                    DoctorStatus::Pass
                },
                detail: format!(
                    "{} on {} ({signal})",
                    snapshot.ssid.as_deref().unwrap_or("associated network"),
                    snapshot.interface.as_deref().unwrap_or("unknown interface")
                ),
                metric_ms: None,
            }
        }
        Ok(snapshot) => DoctorCheck {
            name: "Wi-Fi".to_string(),
            status: DoctorStatus::NotAvailable,
            detail: snapshot.detail,
            metric_ms: None,
        },
        Err(error) => DoctorCheck {
            name: "Wi-Fi".to_string(),
            status: DoctorStatus::NotAvailable,
            detail: format!("Wi-Fi inspection unavailable: {error:#}"),
            metric_ms: None,
        },
    });

    if !selected_is_active {
        checks.push(DoctorCheck {
            name: "HTTPS".to_string(),
            status: DoctorStatus::NotAvailable,
            detail:
                "HTTPS probe skipped because it cannot be bound safely to the selected interface"
                    .to_string(),
            metric_ms: None,
        });
    } else {
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
    }

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

fn scoped_gateway_target(gateway: IpAddr, scope: Option<&str>) -> String {
    match gateway {
        IpAddr::V6(address) if address.is_unicast_link_local() => scope
            .filter(|scope| !scope.is_empty())
            .map_or_else(|| address.to_string(), |scope| format!("{address}%{scope}")),
        _ => gateway.to_string(),
    }
}

fn gateway_ping(gateway: IpAddr, scope: Option<&str>) -> Result<Option<f64>> {
    let target = scoped_gateway_target(gateway, scope);

    #[cfg(target_os = "windows")]
    let output = {
        let mut command = Command::new("ping");
        if gateway.is_ipv6() {
            command.arg("-6");
        }
        command
            .args(["-n", "1", "-w", "1000", &target])
            .output()
            .context("failed to run ping")?
    };

    #[cfg(target_os = "macos")]
    let output = Command::new(if gateway.is_ipv6() { "ping6" } else { "ping" })
        .args(["-c", "1", "-W", "1000", &target])
        .output()
        .context("failed to run ping")?;

    #[cfg(all(unix, not(target_os = "macos")))]
    let output = {
        let mut command = Command::new("ping");
        if gateway.is_ipv6() {
            command.arg("-6");
        }
        command
            .args(["-c", "1", "-W", "1", &target])
            .output()
            .context("failed to run ping")?
    };

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

    fn completed_speedtest() -> TestResult {
        TestResult {
            timestamp: Utc::now(),
            backend: "test".to_string(),
            server: crate::model::ServerInfo {
                host: "example.test".to_string(),
                name: "Test".to_string(),
            },
            latency: crate::model::LatencyResult {
                idle_ms: 10.0,
                jitter_ms: 1.0,
                download_loaded_ms: Some(12.0),
                upload_loaded_ms: Some(13.0),
                packet_loss_percent: None,
            },
            download: crate::model::ThroughputResult {
                mbps: 100.0,
                bytes: 1,
                seconds: 1.0,
            },
            upload: crate::model::ThroughputResult {
                mbps: 50.0,
                bytes: 1,
                seconds: 1.0,
            },
            analysis: None,
        }
    }

    #[test]
    fn parses_common_ping_formats() {
        assert_eq!(parse_ping_time("64 bytes time=1.42 ms"), Some(1.42));
        assert_eq!(parse_ping_time("time<1ms TTL=64"), Some(1.0));
    }

    #[test]
    fn connectivity_failure_takes_priority_over_dns_failure() {
        let mut report = DoctorReport {
            timestamp: Utc::now(),
            interface: Some("eth0".to_string()),
            gateway: None,
            checks: vec![
                DoctorCheck {
                    name: "Default route".to_string(),
                    status: DoctorStatus::Pass,
                    detail: "active interface: eth0".to_string(),
                    metric_ms: None,
                },
                DoctorCheck {
                    name: "IPv4 internet".to_string(),
                    status: DoctorStatus::Fail,
                    detail: "connection failed".to_string(),
                    metric_ms: None,
                },
                DoctorCheck {
                    name: "DNS resolver".to_string(),
                    status: DoctorStatus::Fail,
                    detail: "queries failed".to_string(),
                    metric_ms: None,
                },
            ],
            diagnosis: String::new(),
            recommendation: None,
            speedtest: None,
        };
        report.refresh_diagnosis();
        assert_eq!(report.diagnosis, "IPv4 internet connectivity failed");
    }

    #[test]
    fn missing_default_route_is_reported_first() {
        let mut report = DoctorReport {
            timestamp: Utc::now(),
            interface: Some("dummy0".to_string()),
            gateway: None,
            checks: vec![DoctorCheck {
                name: "Default route".to_string(),
                status: DoctorStatus::Fail,
                detail: "dummy0 has no default route".to_string(),
                metric_ms: None,
            }],
            diagnosis: String::new(),
            recommendation: None,
            speedtest: None,
        };
        report.attach_speedtest_failure(&anyhow::anyhow!("endpoint unavailable"));
        report.refresh_diagnosis();
        assert!(report.diagnosis.contains("no default Internet route"));
        assert!(report.checks.iter().any(|check| {
            check.name == "Full speed test" && check.status == DoctorStatus::NotAvailable
        }));
    }

    #[test]
    fn completed_full_test_is_not_recommended_again() {
        let mut report = DoctorReport {
            timestamp: Utc::now(),
            interface: Some("eth0".to_string()),
            gateway: None,
            checks: Vec::new(),
            diagnosis: String::new(),
            recommendation: None,
            speedtest: None,
        };
        report.attach_speedtest(completed_speedtest());

        assert!(!report
            .recommendation
            .as_deref()
            .unwrap_or_default()
            .contains("doctor --full"));
        assert!(report
            .recommendation
            .as_deref()
            .unwrap_or_default()
            .contains("stability"));
    }

    #[test]
    fn unavailable_full_test_is_reported_as_unavailable() {
        let mut report = DoctorReport {
            timestamp: Utc::now(),
            interface: Some("eth0".to_string()),
            gateway: None,
            checks: Vec::new(),
            diagnosis: String::new(),
            recommendation: None,
            speedtest: None,
        };
        report.attach_speedtest_failure(&anyhow::anyhow!("endpoint unavailable"));

        assert!(report.diagnosis.contains("throughput test was unavailable"));
        assert!(report
            .recommendation
            .as_deref()
            .unwrap_or_default()
            .contains("Retry"));
    }

    #[test]
    fn scopes_ipv6_link_local_gateway_targets() {
        assert_eq!(
            scoped_gateway_target("fe80::1".parse().unwrap(), Some("en0")),
            "fe80::1%en0"
        );
        assert_eq!(
            scoped_gateway_target("2001:db8::1".parse().unwrap(), Some("en0")),
            "2001:db8::1"
        );
        assert_eq!(
            scoped_gateway_target("192.0.2.1".parse().unwrap(), Some("en0")),
            "192.0.2.1"
        );
    }
}
