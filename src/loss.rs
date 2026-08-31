use std::{net::IpAddr, process::Output, time::Duration};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::{net::lookup_host, process::Command, time::timeout};

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
    let validated_target = validate_target(target)?;
    ensure_target_resolves(validated_target).await?;
    let target = validated_target.to_string();
    let packets_sent = usize::from(count.clamp(3, 200));
    let output = run_ping(validated_target, packets_sent, suggested_timeout(count)).await?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let samples = parse_reply_times(&stdout);
    let complete_packet_loss = reports_complete_packet_loss(&stdout, &stderr)
        && !contains_fatal_ping_diagnostic(&stdout, &stderr);

    if samples.is_empty() && !complete_packet_loss {
        let detail = command_failure_detail(&stdout, &stderr);
        if output.status.success() {
            anyhow::bail!(
                "ping completed successfully, but no reply latency could be parsed: {detail}"
            );
        }
        anyhow::bail!(
            "ping exited with status {} before producing a valid measurement: {detail}",
            output.status
        );
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

fn validate_target(target: &str) -> Result<&str> {
    if target.is_empty() {
        anyhow::bail!("ping target cannot be empty");
    }
    if target != target.trim() || target.chars().any(char::is_whitespace) {
        anyhow::bail!("ping target must not contain whitespace");
    }
    if target.starts_with('-') {
        anyhow::bail!("ping target must not start with an option prefix (`-`)");
    }
    if target.parse::<IpAddr>().is_ok() {
        return Ok(target);
    }
    if !target.is_ascii() {
        anyhow::bail!("ping target must be an IP address or an ASCII hostname");
    }

    let hostname = target.strip_suffix('.').unwrap_or(target);
    if hostname.is_empty() || hostname.len() > 253 {
        anyhow::bail!("ping target is not a valid hostname");
    }
    for label in hostname.split('.') {
        let valid_length = !label.is_empty() && label.len() <= 63;
        let valid_edges = label
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
            && label
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric);
        let valid_characters = label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
        if !(valid_length && valid_edges && valid_characters) {
            anyhow::bail!("ping target `{target}` is not a valid hostname or IP address");
        }
    }
    Ok(target)
}

async fn ensure_target_resolves(target: &str) -> Result<()> {
    if target.parse::<IpAddr>().is_ok() {
        return Ok(());
    }

    let addresses = timeout(Duration::from_secs(10), lookup_host((target, 0)))
        .await
        .with_context(|| format!("timed out resolving ping target `{target}`"))?
        .with_context(|| format!("failed to resolve ping target `{target}`"))?;
    if addresses.into_iter().next().is_none() {
        anyhow::bail!("ping target `{target}` resolved to no IP addresses");
    }
    Ok(())
}

async fn run_ping(target: &str, count: usize, deadline: Duration) -> Result<Output> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("ping");
        command
            .args(["-n", &count.to_string(), "-w", "1000"])
            .arg(target);
        command
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("ping");
        command
            .args(["-c", &count.to_string(), "-W", "1000", "-i", "0.2"])
            .arg(target);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("ping");
        command
            .args(["-c", &count.to_string(), "-W", "1", "-i", "0.2"])
            .arg(target);
        command
    };

    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    anyhow::bail!("ICMP packet-loss measurement is not supported on this platform");

    command.kill_on_drop(true);
    match timeout(deadline, command.output()).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => Err(error).context("failed to start or wait for the ping command"),
        Err(_) => anyhow::bail!(
            "ping command exceeded its {} second measurement deadline",
            deadline.as_secs()
        ),
    }
}

fn parse_reply_times(text: &str) -> Vec<f64> {
    text.lines().filter_map(parse_reply_time).collect()
}

fn parse_reply_time(line: &str) -> Option<f64> {
    let lowercase = line.to_ascii_lowercase();
    let looks_like_reply = lowercase.contains("ttl=")
        || lowercase.contains("hlim=")
        || lowercase.contains("icmp_seq=")
        || ["time=", "time<", "zeit=", "zeit<", "temps=", "temps<"]
            .into_iter()
            .any(|marker| lowercase.contains(marker));
    if !looks_like_reply {
        return None;
    }

    let unit_positions = lowercase
        .match_indices("ms")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    for unit_index in unit_positions.into_iter().rev() {
        let before_unit = line[..unit_index].trim_end();
        let bytes = before_unit.as_bytes();
        let mut number_start = bytes.len();
        while number_start > 0
            && (bytes[number_start - 1].is_ascii_digit()
                || matches!(bytes[number_start - 1], b'.' | b','))
        {
            number_start -= 1;
        }
        if number_start == bytes.len() {
            continue;
        }

        let number = before_unit[number_start..].replace(',', ".");
        let parsed = number.parse::<f64>().ok()?;
        let is_upper_bound = before_unit[..number_start].trim_end().ends_with('<');
        return Some(if is_upper_bound {
            (parsed / 2.0).max(0.1)
        } else {
            parsed
        });
    }
    None
}

fn reports_complete_packet_loss(stdout: &str, stderr: &str) -> bool {
    [stdout, stderr].into_iter().any(|text| {
        text.split('%').any(|prefix| {
            let prefix = prefix.trim_end();
            let number_reversed = prefix
                .chars()
                .rev()
                .take_while(|character| {
                    character.is_ascii_digit() || matches!(character, '.' | ',')
                })
                .collect::<String>();
            let number = number_reversed.chars().rev().collect::<String>();
            number
                .replace(',', ".")
                .parse::<f64>()
                .is_ok_and(|percent| (percent - 100.0).abs() < f64::EPSILON)
        })
    })
}

fn contains_fatal_ping_diagnostic(stdout: &str, stderr: &str) -> bool {
    let text = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    [
        "general failure",
        "transmit failed",
        "operation not permitted",
        "permission denied",
        "could not find host",
        "unknown host",
        "cannot resolve",
        "name or service not known",
        "network is unreachable",
        "invalid argument",
        "invalid option",
        "usage: ping",
    ]
    .into_iter()
    .any(|diagnostic| text.contains(diagnostic))
}

fn command_failure_detail(stdout: &str, stderr: &str) -> String {
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    if detail.is_empty() {
        return "the command produced no diagnostic output".to_string();
    }
    detail
        .replace(['\r', '\n'], " ")
        .chars()
        .take(500)
        .collect()
}

pub fn default_target() -> &'static str {
    "1.1.1.1"
}

pub fn suggested_timeout(count: u16) -> Duration {
    Duration::from_secs(u64::from(count.clamp(3, 200)) * 2 + 5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unix_windows_and_localized_reply_times() {
        let sample = "64 bytes from 1.1.1.1: icmp_seq=1 ttl=57 time=8.42 ms\n\
Reply from 1.1.1.1: bytes=32 time=12ms TTL=57\n\
Reply from 1.1.1.1: bytes=32 time<1ms TTL=57\n\
Reply from ::1: time<1ms\n\
Antwort von ::1: Zeit<1ms\n\
Antwort von 1.1.1.1: Bytes=32 Zeit=9,25ms TTL=57\n\
Réponse de 1.1.1.1 : octets=32 temps=7 ms TTL=57\n\
rtt min/avg/max/mdev = 7.000/8.000/9.250/1.000 ms";
        let values = parse_reply_times(sample);
        assert_eq!(values.len(), 7);
        assert!((values[0] - 8.42).abs() < 0.001);
        assert_eq!(values[1], 12.0);
        assert_eq!(values[2], 0.5);
        assert_eq!(values[3], 0.5);
        assert_eq!(values[4], 0.5);
        assert_eq!(values[5], 9.25);
        assert_eq!(values[6], 7.0);
    }

    #[test]
    fn reply_parser_uses_the_latency_field_not_ms_in_a_hostname() {
        let line = "64 bytes from 123ms.example (1.2.3.4): icmp_seq=1 ttl=57 time=8.42 ms";
        assert_eq!(parse_reply_time(line), Some(8.42));
    }

    #[test]
    fn rejects_option_like_and_invalid_targets() {
        assert!(validate_target("--help").is_err());
        assert!(validate_target("-c").is_err());
        assert!(validate_target("example.com extra").is_err());
        assert!(validate_target("https://example.com").is_err());
        assert!(validate_target("bad_label.example").is_err());
        assert!(validate_target("").is_err());

        assert_eq!(validate_target("1.1.1.1").unwrap(), "1.1.1.1");
        assert_eq!(validate_target("::1").unwrap(), "::1");
        assert_eq!(validate_target("example.com.").unwrap(), "example.com.");
    }

    #[test]
    fn recognizes_localized_complete_loss_summary() {
        assert!(reports_complete_packet_loss(
            "Pakete: Gesendet = 3, Empfangen = 0, Verloren = 3 (100 % Verlust)",
            ""
        ));
        assert!(!reports_complete_packet_loss(
            "3 packets transmitted, 2 received, 33.3% packet loss",
            ""
        ));
        assert!(contains_fatal_ping_diagnostic(
            "PING: transmit failed. General failure.\nPackets: Sent = 3, Received = 0, Lost = 3 (100% loss)",
            ""
        ));
    }
}
