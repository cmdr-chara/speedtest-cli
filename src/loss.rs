use std::time::Duration;
use tokio::process::Command;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{analysis, model::LatencyDistribution};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacketLossResult {
    pub timestamp: DateTime<Utc>,
    pub target: String,
    pub protocol: String,
    pub packets_sent: usize,
    pub packets_received: usize,
    pub packets_lost: usize,
    pub loss_percent: f64,
    pub rtt: Option<LatencyDistribution>,
    pub caveat: String,
}

impl PacketLossResult {
    pub fn pretty_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

pub async fn measure(target: &str, count: u16) -> Result<PacketLossResult> {
    let target = validate_target(target).map_err(anyhow::Error::msg)?;
    let packets_sent = usize::from(count.clamp(3, 200));
    let output = tokio::time::timeout(suggested_timeout(count), run_ping(&target, packets_sent))
        .await
        .context("ICMP command timed out")??;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let samples = parse_reply_times(&stdout);

    if samples.is_empty() && (!stderr.trim().is_empty() || !recognized_zero_replies(&stdout)) {
        anyhow::bail!("ping did not return a supported reply/summary format; check the target and native ping availability (localized Windows output may be unsupported)");
    }

    let packets_received = samples.len().min(packets_sent);
    let packets_lost = packets_sent.saturating_sub(packets_received);
    let loss_percent = if packets_sent == 0 {
        0.0
    } else {
        packets_lost as f64 / packets_sent as f64 * 100.0
    };

    Ok(PacketLossResult {
        timestamp: Utc::now(),
        target,
        protocol: "icmp_echo".to_string(),
        packets_sent,
        packets_received,
        packets_lost,
        loss_percent,
        rtt: analysis::distribution(&samples),
        caveat: "ICMP echo loss is a real response-loss measurement, but a host or firewall that deprioritizes/blocks ICMP can look lossy even when application traffic is healthy.".to_string(),
    })
}

async fn run_ping(target: &str, count: usize) -> Result<std::process::Output> {
    #[cfg(target_os = "windows")]
    let output = Command::new("ping")
        .args(["-n", &count.to_string(), "-w", "1000", target])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .kill_on_drop(true)
        .output()
        .await
        .context("failed to run Windows ping")?;

    #[cfg(target_os = "macos")]
    let output = Command::new("ping")
        .args(["-c", &count.to_string(), "-W", "1000", "-i", "0.2", target])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .kill_on_drop(true)
        .output()
        .await
        .context("failed to run macOS ping")?;

    #[cfg(all(unix, not(target_os = "macos")))]
    let output = Command::new("ping")
        .args(["-c", &count.to_string(), "-W", "1", "-i", "0.2", target])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .kill_on_drop(true)
        .output()
        .await
        .context("failed to run ping")?;

    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    anyhow::bail!("ICMP packet-loss measurement is not supported on this platform");

    Ok(output)
}

/// Reject option-shaped and ambiguous targets before invoking a native program.
pub fn validate_target(target: &str) -> Result<String, String> {
    if target.parse::<std::net::IpAddr>().is_ok() {
        return Ok(target.to_string());
    }
    let host = target.strip_suffix('.').unwrap_or(target);
    if host.is_empty()
        || host.len() > 253
        || !host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return Err(
            "target must be an IP address or ASCII DNS hostname, not an option or command"
                .to_string(),
        );
    }
    Ok(target.to_string())
}

fn recognized_zero_replies(text: &str) -> bool {
    text.contains("0 received")
        || text.contains("0 packets received")
        || text.contains("Received = 0")
}

fn parse_reply_times(text: &str) -> Vec<f64> {
    text.lines().filter_map(parse_reply_time).collect()
}

fn parse_reply_time(line: &str) -> Option<f64> {
    ["time=", "time<"].into_iter().find_map(|marker| {
        let (_, tail) = line.split_once(marker)?;
        let value = tail
            .chars()
            .take_while(|character| character.is_ascii_digit() || *character == '.')
            .collect::<String>();
        let parsed = value.parse::<f64>().ok()?;
        if !parsed.is_finite() {
            return None;
        }
        Some(if marker == "time<" {
            (parsed / 2.0).max(0.1)
        } else {
            parsed
        })
    })
}

pub fn default_target() -> &'static str {
    "1.1.1.1"
}

pub fn suggested_timeout(count: u16) -> Duration {
    Duration::from_secs(u64::from(count.clamp(3, 200)) + 5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_native_option_injection_and_accepts_addresses() {
        for target in [
            "-f",
            "--help",
            "a b",
            "host\n",
            "$(whoami)",
            "a..b",
            "-c9999",
        ] {
            assert!(validate_target(target).is_err(), "{target}");
        }
        for target in [
            "1.1.1.1",
            "::1",
            "speed.example.test",
            "localhost",
            "example.test.",
        ] {
            assert!(validate_target(target).is_ok(), "{target}");
        }
        assert!(!recognized_zero_replies("Usage: ping [options]"));
    }

    #[test]
    fn parses_unix_and_windows_reply_times() {
        let sample = "64 bytes from 1.1.1.1: icmp_seq=1 ttl=57 time=8.42 ms\nReply from 1.1.1.1: bytes=32 time=12ms TTL=57\nReply from 1.1.1.1: bytes=32 time<1ms TTL=57";
        let values = parse_reply_times(sample);
        assert_eq!(values.len(), 3);
        assert!((values[0] - 8.42).abs() < 0.001);
        assert_eq!(values[1], 12.0);
        assert_eq!(values[2], 0.5);
    }
}
