//! Cross-platform path diagnostics used by the guided health assessment.
//!
//! These probes are explicit network operations. Constructors remain pure and
//! the caller owns their cancellation scope.

use std::{net::IpAddr, time::Duration};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::{
    net::{lookup_host, TcpStream},
    process::Command,
    time::{timeout, Instant},
};

use crate::transport::{self, TransportProbe, TransportProtocol};
use crate::{dns, engine::AddressFamily};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedNetworkReport {
    pub timestamp: DateTime<Utc>,
    pub family: String,
    pub vpn: VpnAssessment,
    pub handshake: Option<HandshakeProbe>,
    pub mtu: Option<MtuProbe>,
    pub transport: Vec<TransportProbe>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpnAssessment {
    pub likely: bool,
    pub interface: Option<String>,
    pub signals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeProbe {
    pub host: String,
    pub address: IpAddr,
    pub dns_ms: f64,
    pub tcp_ms: f64,
    pub https_ms: f64,
    pub http_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MtuProbe {
    pub target: IpAddr,
    pub payload_bytes: u16,
    pub path_mtu_bytes: u16,
    pub method: String,
}

pub async fn run(family: AddressFamily, interface: Option<&str>) -> AdvancedNetworkReport {
    let detected_interface = interface
        .map(str::to_string)
        .or_else(|| dns::system::inspect(None).ok().map(|state| state.interface));
    let vpn = detect_vpn(detected_interface.as_deref());
    let handshake = handshake_probe(family).await.ok();
    let mtu = probe_mtu(family).await;
    let (http2, http3) = tokio::join!(
        transport::probe(family, TransportProtocol::Http2),
        transport::probe(family, TransportProtocol::Http3),
    );
    let transport = vec![http2, http3];
    AdvancedNetworkReport {
        timestamp: Utc::now(),
        family: family.label().to_string(),
        vpn,
        handshake,
        mtu,
        transport,
    }
}

pub fn detect_vpn(interface: Option<&str>) -> VpnAssessment {
    let name = interface.map(str::to_ascii_lowercase);
    let mut signals = Vec::new();
    if let Some(name) = &name {
        let markers = [
            ("tun", "TUN interface"),
            ("tap", "TAP interface"),
            ("wg", "WireGuard interface"),
            ("utun", "macOS tunnel interface"),
            ("ppp", "PPP interface"),
            ("vpn", "VPN-named interface"),
            ("tailscale", "Tailscale interface"),
            ("zt", "ZeroTier-style interface"),
        ];
        for (marker, label) in markers {
            if name == marker || name.starts_with(marker) || name.contains(&format!("-{marker}")) {
                signals.push(label.to_string());
            }
        }
    }
    VpnAssessment {
        likely: !signals.is_empty(),
        interface: interface.map(str::to_string),
        signals,
    }
}

async fn handshake_probe(family: AddressFamily) -> Result<HandshakeProbe> {
    const HOST: &str = "example.com";
    let dns_started = Instant::now();
    let addresses = timeout(HANDSHAKE_TIMEOUT, lookup_host((HOST, 443)))
        .await
        .context("DNS lookup timed out")?
        .context("DNS lookup failed")?
        .filter(|address| match family {
            AddressFamily::Any => true,
            AddressFamily::Ipv4 => address.is_ipv4(),
            AddressFamily::Ipv6 => address.is_ipv6(),
        })
        .collect::<Vec<_>>();
    let address = addresses
        .first()
        .context("no address for requested family")?
        .ip();
    let dns_ms = dns_started.elapsed().as_secs_f64() * 1000.0;

    let tcp_started = Instant::now();
    timeout(HANDSHAKE_TIMEOUT, TcpStream::connect((address, 443)))
        .await
        .context("TCP connection timed out")?
        .context("TCP connection failed")?;
    let tcp_ms = tcp_started.elapsed().as_secs_f64() * 1000.0;

    let client = Client::builder()
        .user_agent(concat!("speedtest-cli/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .local_address(family.local_address())
        .build()
        .context("failed to build handshake client")?;
    let https_started = Instant::now();
    let response = client
        .get(format!("https://{HOST}/"))
        .send()
        .await
        .context("HTTPS request failed")?;
    let http_version = format!("{:?}", response.version());
    let _ = response.bytes().await;
    Ok(HandshakeProbe {
        host: HOST.to_string(),
        address,
        dns_ms,
        tcp_ms,
        https_ms: https_started.elapsed().as_secs_f64() * 1000.0,
        http_version,
    })
}

async fn probe_mtu(family: AddressFamily) -> Option<MtuProbe> {
    let (target, low, high, header, method) = match family {
        AddressFamily::Ipv6 => (
            "2606:4700:4700::1111".parse().ok()?,
            512_u16,
            1452_u16,
            48_u16,
            "ICMPv6 packet-too-big search",
        ),
        AddressFamily::Any | AddressFamily::Ipv4 => (
            "1.1.1.1".parse().ok()?,
            512_u16,
            1472_u16,
            28_u16,
            "IPv4 DF packet-too-big search",
        ),
    };
    if !ping_payload(target, family, low).await {
        return None;
    }
    let mut lower = low;
    let mut upper = high;
    while lower < upper {
        let middle = lower.saturating_add(upper.saturating_sub(lower).div_ceil(2));
        if ping_payload(target, family, middle).await {
            lower = middle;
        } else {
            upper = middle.saturating_sub(1);
        }
    }
    Some(MtuProbe {
        target,
        payload_bytes: lower,
        path_mtu_bytes: lower.saturating_add(header),
        method: method.to_string(),
    })
}

async fn ping_payload(target: IpAddr, family: AddressFamily, payload: u16) -> bool {
    let target = target.to_string();
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("ping");
        command.args([
            if matches!(family, AddressFamily::Ipv6) {
                "-6"
            } else {
                "-4"
            },
            "-f",
            "-n",
            "1",
            "-w",
            "1000",
            "-l",
            &payload.to_string(),
            &target,
        ]);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new(if matches!(family, AddressFamily::Ipv6) {
            "ping6"
        } else {
            "ping"
        });
        command.args([
            "-D",
            "-c",
            "1",
            "-W",
            "1000",
            "-s",
            &payload.to_string(),
            &target,
        ]);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("ping");
        if matches!(family, AddressFamily::Ipv6) {
            command.arg("-6");
        }
        command.args([
            "-M",
            "do",
            "-c",
            "1",
            "-W",
            "1",
            "-s",
            &payload.to_string(),
            &target,
        ]);
        command
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    return false;

    #[cfg(any(target_os = "windows", target_os = "macos", unix))]
    {
        command.kill_on_drop(true);
        timeout(Duration::from_secs(2), command.output())
            .await
            .is_ok_and(|result| result.is_ok_and(|output| output.status.success()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_tunnel_interfaces_without_claiming_every_interface_is_a_vpn() {
        assert!(detect_vpn(Some("wg0")).likely);
        assert!(detect_vpn(Some("utun3")).likely);
        assert!(!detect_vpn(Some("enp4s0")).likely);
        assert!(detect_vpn(Some("wg0")).signals[0].contains("WireGuard"));
    }
}
