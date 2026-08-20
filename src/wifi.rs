use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiSnapshot {
    pub available: bool,
    pub interface: Option<String>,
    pub ssid: Option<String>,
    pub signal_percent: Option<f64>,
    pub signal_dbm: Option<f64>,
    pub band: Option<String>,
    pub channel: Option<u32>,
    pub link_mbps: Option<f64>,
    pub radio: Option<String>,
    pub detail: String,
}

impl WifiSnapshot {
    pub fn pretty_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

pub fn inspect(interface: Option<&str>) -> Result<WifiSnapshot> {
    #[cfg(target_os = "windows")]
    {
        return inspect_windows(interface);
    }
    #[cfg(target_os = "macos")]
    {
        return inspect_macos(interface);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return inspect_linux(interface);
    }
    #[allow(unreachable_code)]
    Ok(unavailable(
        "Wi-Fi diagnostics are not implemented on this platform",
    ))
}

fn unavailable(detail: impl Into<String>) -> WifiSnapshot {
    WifiSnapshot {
        available: false,
        interface: None,
        ssid: None,
        signal_percent: None,
        signal_dbm: None,
        band: None,
        channel: None,
        link_mbps: None,
        radio: None,
        detail: detail.into(),
    }
}

#[cfg(target_os = "windows")]
fn inspect_windows(interface: Option<&str>) -> Result<WifiSnapshot> {
    let output = Command::new("netsh")
        .args(["wlan", "show", "interfaces"])
        .output()
        .context("failed to run `netsh wlan show interfaces`")?;
    if !output.status.success() {
        return Ok(unavailable(
            "Windows WLAN service did not expose an active Wi-Fi interface",
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let requested = interface.map(str::to_ascii_lowercase);
    let mut selected = Vec::new();
    let mut current = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("Name") && line.contains(':') && !current.is_empty() {
            if block_matches(&current, requested.as_deref()) {
                selected = std::mem::take(&mut current);
                break;
            }
            current = Vec::new();
        }
        current.push(line.to_string());
    }
    if selected.is_empty() && block_matches(&current, requested.as_deref()) {
        selected = current;
    }
    if selected.is_empty() {
        selected = text.lines().map(ToString::to_string).collect();
    }

    let block = selected.join("\n");
    let interface = field(&block, "Name");
    let ssid = field(&block, "SSID").filter(|value| !value.eq_ignore_ascii_case("n/a"));
    let signal_percent = field(&block, "Signal")
        .and_then(|value| value.trim_end_matches('%').trim().parse::<f64>().ok());
    let signal_dbm = signal_percent.map(|quality| quality / 2.0 - 100.0);
    let channel = field(&block, "Channel").and_then(|value| value.parse::<u32>().ok());
    let radio = field(&block, "Radio type");
    let receive = field(&block, "Receive rate (Mbps)").and_then(|value| value.parse::<f64>().ok());
    let transmit =
        field(&block, "Transmit rate (Mbps)").and_then(|value| value.parse::<f64>().ok());
    let link_mbps = match (receive, transmit) {
        (Some(rx), Some(tx)) => Some(rx.min(tx)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        _ => None,
    };

    Ok(WifiSnapshot {
        available: ssid.is_some(),
        interface,
        ssid,
        signal_percent,
        signal_dbm,
        band: channel.and_then(band_from_channel),
        channel,
        link_mbps,
        radio,
        detail: if signal_percent.is_some() {
            "Signal dBm is estimated from the Windows WLAN quality percentage; link rate is not Internet throughput.".to_string()
        } else {
            "Windows WLAN interface detected; signal details were incomplete.".to_string()
        },
    })
}

#[cfg(target_os = "windows")]
fn block_matches(lines: &[String], requested: Option<&str>) -> bool {
    let Some(requested) = requested else {
        return true;
    };
    let text = lines.join("\n").to_ascii_lowercase();
    text.lines().any(|line| {
        line.split_once(':')
            .is_some_and(|(key, value)| key.trim() == "name" && value.trim() == requested)
    })
}

#[cfg(target_os = "macos")]
fn inspect_macos(interface: Option<&str>) -> Result<WifiSnapshot> {
    let device = interface.unwrap_or("en0");
    let airport_path =
        "/System/Library/PrivateFrameworks/Apple80211.framework/Versions/Current/Resources/airport";
    if std::path::Path::new(airport_path).exists() {
        let output = Command::new(airport_path)
            .arg("-I")
            .output()
            .context("failed to query macOS AirPort diagnostics")?;
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let ssid = colon_field(&text, "SSID");
            let signal_dbm = colon_field(&text, "agrCtlRSSI").and_then(|value| value.parse().ok());
            let channel_text = colon_field(&text, "channel");
            let channel = channel_text
                .as_deref()
                .and_then(|value| value.split(',').next())
                .and_then(|value| value.trim().parse::<u32>().ok());
            let link_mbps = colon_field(&text, "lastTxRate").and_then(|value| value.parse().ok());
            return Ok(WifiSnapshot {
                available: ssid.is_some(),
                interface: Some(device.to_string()),
                ssid,
                signal_percent: signal_dbm.map(dbm_to_percent),
                signal_dbm,
                band: channel.and_then(band_from_channel),
                channel,
                link_mbps,
                radio: None,
                detail: "macOS AirPort diagnostic data; link rate is not Internet throughput."
                    .to_string(),
            });
        }
    }

    let output = Command::new("networksetup")
        .args(["-getairportnetwork", device])
        .output()
        .context("failed to query macOS Wi-Fi network")?;
    if !output.status.success() {
        return Ok(unavailable("macOS did not expose an active Wi-Fi network"));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let ssid = text
        .split_once(':')
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty());
    Ok(WifiSnapshot {
        available: ssid.is_some(),
        interface: Some(device.to_string()),
        ssid,
        signal_percent: None,
        signal_dbm: None,
        band: None,
        channel: None,
        link_mbps: None,
        radio: None,
        detail: "macOS exposed the associated SSID but detailed radio metrics were unavailable without a supported AirPort diagnostic interface.".to_string(),
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn inspect_linux(interface: Option<&str>) -> Result<WifiSnapshot> {
    let device = if let Some(interface) = interface {
        interface.to_string()
    } else {
        let output = Command::new("iw")
            .arg("dev")
            .output()
            .context("`iw` is required for Linux Wi-Fi diagnostics")?;
        let text = String::from_utf8_lossy(&output.stdout);
        text.lines()
            .find_map(|line| line.trim().strip_prefix("Interface ").map(str::to_string))
            .ok_or_else(|| anyhow::anyhow!("no Linux wireless interface was found"))?
    };

    let link = Command::new("iw")
        .args(["dev", &device, "link"])
        .output()
        .context("failed to query Linux Wi-Fi link")?;
    if !link.status.success() {
        return Ok(unavailable(format!(
            "{device} is not associated with a Wi-Fi network"
        )));
    }
    let text = String::from_utf8_lossy(&link.stdout);
    let ssid = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("SSID: ").map(str::to_string));
    let signal_dbm = text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("signal: ")
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<f64>().ok())
    });
    let link_mbps = text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("tx bitrate: ")
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<f64>().ok())
    });
    let frequency = text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("freq: ")
            .and_then(|value| value.parse::<u32>().ok())
    });
    let band = frequency.map(|mhz| {
        match mhz {
            2400..=2500 => "2.4 GHz",
            4900..=5900 => "5 GHz",
            5925..=7125 => "6 GHz",
            _ => "unknown",
        }
        .to_string()
    });

    let info = Command::new("iw")
        .args(["dev", &device, "info"])
        .output()
        .ok();
    let channel = info.as_ref().and_then(|output| {
        let text = String::from_utf8_lossy(&output.stdout);
        text.lines().find_map(|line| {
            let line = line.trim();
            let (_, tail) = line.split_once("channel ")?;
            tail.split_whitespace().next()?.parse::<u32>().ok()
        })
    });

    Ok(WifiSnapshot {
        available: ssid.is_some(),
        interface: Some(device),
        ssid,
        signal_percent: signal_dbm.map(dbm_to_percent),
        signal_dbm,
        band,
        channel,
        link_mbps,
        radio: None,
        detail: "Linux `iw` link diagnostics; PHY/link rate is not Internet throughput."
            .to_string(),
    })
}

#[cfg(any(test, unix))]
fn dbm_to_percent(dbm: f64) -> f64 {
    ((dbm + 100.0) * 2.0).clamp(0.0, 100.0)
}

#[cfg(any(test, target_os = "windows", target_os = "macos"))]
fn band_from_channel(channel: u32) -> Option<String> {
    match channel {
        1..=14 => Some("2.4 GHz".to_string()),
        32..=177 => Some("5 GHz".to_string()),
        1_000.. => None,
        _ => Some("6 GHz / platform-specific".to_string()),
    }
}

#[cfg(target_os = "windows")]
fn field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (left, right) = line.split_once(':')?;
        (left.trim() == key).then(|| right.trim().to_string())
    })
}

#[cfg(target_os = "macos")]
fn colon_field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (left, right) = line.split_once(':')?;
        (left.trim() == key).then(|| right.trim().to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dbm_quality_conversion_is_bounded() {
        assert_eq!(dbm_to_percent(-100.0), 0.0);
        assert_eq!(dbm_to_percent(-50.0), 100.0);
        assert_eq!(dbm_to_percent(-200.0), 0.0);
    }

    #[test]
    fn common_channels_map_to_bands() {
        assert_eq!(band_from_channel(6).as_deref(), Some("2.4 GHz"));
        assert_eq!(band_from_channel(44).as_deref(), Some("5 GHz"));
    }
}
