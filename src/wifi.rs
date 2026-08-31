use std::process::{Command, Output};

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
        return Ok(unavailable(format!(
            "Windows WLAN service did not expose an active Wi-Fi interface ({}): {}",
            output.status,
            output_diagnostic(&output)
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let Some(block) = select_windows_interface_block(&text, interface)? else {
        return Ok(unavailable(
            "Windows WLAN output did not contain a Wi-Fi interface block",
        ));
    };

    let interface = field(&block, "Name");
    let ssid = field(&block, "SSID").filter(|value| !value.eq_ignore_ascii_case("n/a"));
    let signal_percent = field(&block, "Signal")
        .and_then(|value| value.trim_end_matches('%').trim().parse::<f64>().ok());
    let signal_dbm = signal_percent.map(|quality| quality / 2.0 - 100.0);
    let channel = field(&block, "Channel").and_then(|value| value.parse::<u32>().ok());
    let band = field(&block, "Band")
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("n/a"))
        .or_else(|| channel.and_then(band_from_channel));
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
        band,
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

#[cfg(any(test, target_os = "windows"))]
fn windows_interface_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    for line in text.lines() {
        let starts_block = line
            .split_once(':')
            .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case("Name"));
        if starts_block {
            if !current.is_empty() {
                blocks.push(current.join("\n"));
            }
            current = vec![line.to_string()];
        } else if !current.is_empty() {
            current.push(line.to_string());
        }
    }
    if !current.is_empty() {
        blocks.push(current.join("\n"));
    }
    blocks
}

#[cfg(any(test, target_os = "windows"))]
fn select_windows_interface_block(text: &str, requested: Option<&str>) -> Result<Option<String>> {
    let blocks = windows_interface_blocks(text);
    let Some(requested) = requested else {
        return Ok(blocks
            .iter()
            .find(|block| {
                field(block, "SSID")
                    .is_some_and(|ssid| !ssid.is_empty() && !ssid.eq_ignore_ascii_case("n/a"))
            })
            .cloned()
            .or_else(|| blocks.into_iter().next()));
    };

    if let Some(block) = blocks
        .iter()
        .find(|block| field(block, "Name").is_some_and(|name| name.eq_ignore_ascii_case(requested)))
    {
        return Ok(Some(block.clone()));
    }

    let available = blocks
        .iter()
        .filter_map(|block| field(block, "Name"))
        .collect::<Vec<_>>();
    let available = if available.is_empty() {
        "none".to_string()
    } else {
        available.join(", ")
    };
    anyhow::bail!(
        "Windows Wi-Fi interface `{requested}` was not found (available interfaces: {available})"
    )
}

#[cfg(target_os = "macos")]
fn inspect_macos(interface: Option<&str>) -> Result<WifiSnapshot> {
    let hardware_ports = Command::new("networksetup")
        .arg("-listallhardwareports")
        .output()
        .context("failed to list macOS network hardware ports")?;
    if !hardware_ports.status.success() {
        anyhow::bail!(
            "`networksetup -listallhardwareports` exited with status {}: {}",
            hardware_ports.status,
            output_diagnostic(&hardware_ports)
        );
    }
    let hardware_port_text = String::from_utf8_lossy(&hardware_ports.stdout);
    let wifi_devices = parse_macos_wifi_devices(&hardware_port_text);
    let device = if interface.is_some() {
        select_macos_wifi_device(&wifi_devices, interface)?
    } else {
        wifi_devices
            .iter()
            .find(|candidate| {
                Command::new("networksetup")
                    .args(["-getairportnetwork", candidate.as_str()])
                    .output()
                    .is_ok_and(|output| {
                        output.status.success()
                            && macos_network_ssid(&String::from_utf8_lossy(&output.stdout))
                                .is_some()
                    })
            })
            .cloned()
            .or_else(|| wifi_devices.first().cloned())
            .ok_or_else(|| anyhow::anyhow!("no macOS Wi-Fi hardware port was found"))?
    };

    let airport_path =
        "/System/Library/PrivateFrameworks/Apple80211.framework/Versions/Current/Resources/airport";
    if wifi_devices.len() == 1 && std::path::Path::new(airport_path).exists() {
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
            if ssid.is_some() {
                return Ok(WifiSnapshot {
                    available: true,
                    interface: Some(device),
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
    }

    let output = Command::new("networksetup")
        .args(["-getairportnetwork", &device])
        .output()
        .context("failed to query macOS Wi-Fi network")?;
    if !output.status.success() {
        return Ok(unavailable(format!(
            "macOS did not expose an active Wi-Fi network on {device} ({}): {}",
            output.status,
            output_diagnostic(&output)
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let ssid = macos_network_ssid(&text);
    Ok(WifiSnapshot {
        available: ssid.is_some(),
        interface: Some(device),
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

#[cfg(any(test, target_os = "macos"))]
fn parse_macos_wifi_devices(text: &str) -> Vec<String> {
    let mut devices = Vec::new();
    let mut wifi_port = false;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "Hardware Port" => {
                let port = value.trim().to_ascii_lowercase();
                wifi_port = matches!(port.as_str(), "wi-fi" | "wifi" | "airport");
            }
            "Device" if wifi_port => {
                let device = value.trim();
                if !device.is_empty() && !devices.iter().any(|known| known == device) {
                    devices.push(device.to_string());
                }
            }
            _ => {}
        }
    }
    devices
}

#[cfg(any(test, target_os = "macos"))]
fn select_macos_wifi_device(devices: &[String], requested: Option<&str>) -> Result<String> {
    if let Some(requested) = requested {
        if devices.iter().any(|device| device == requested) {
            return Ok(requested.to_string());
        }
        let available = if devices.is_empty() {
            "none".to_string()
        } else {
            devices.join(", ")
        };
        anyhow::bail!(
            "macOS interface `{requested}` is not a Wi-Fi hardware device (available Wi-Fi devices: {available})"
        );
    }
    devices
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no macOS Wi-Fi hardware port was found"))
}

#[cfg(any(test, target_os = "macos"))]
fn macos_network_ssid(text: &str) -> Option<String> {
    if text
        .to_ascii_lowercase()
        .contains("not associated with an airport network")
    {
        return None;
    }
    text.split_once(':')
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn inspect_linux(interface: Option<&str>) -> Result<WifiSnapshot> {
    let inventory = Command::new("iw")
        .arg("dev")
        .output()
        .context("`iw` is required for Linux Wi-Fi diagnostics")?;
    if !inventory.status.success() {
        anyhow::bail!(
            "`iw dev` exited with status {}: {}",
            inventory.status,
            output_diagnostic(&inventory)
        );
    }
    let inventory_text = String::from_utf8_lossy(&inventory.stdout);
    let interfaces = parse_iw_interfaces(&inventory_text);
    if interfaces.is_empty() {
        anyhow::bail!(
            "no managed Linux Wi-Fi interface was reported by `iw dev`: {}",
            output_diagnostic(&inventory)
        );
    }

    let device = if let Some(requested) = interface {
        if !interfaces.iter().any(|candidate| candidate == requested) {
            anyhow::bail!(
                "`{requested}` is not a managed Linux Wi-Fi interface reported by `iw dev` (available interfaces: {})",
                interfaces.join(", ")
            );
        }
        requested.to_string()
    } else {
        interfaces
            .iter()
            .find(|candidate| {
                Command::new("iw")
                    .args(["dev", candidate.as_str(), "link"])
                    .output()
                    .is_ok_and(|output| {
                        output.status.success()
                            && linux_link_ssid(&String::from_utf8_lossy(&output.stdout)).is_some()
                    })
            })
            .unwrap_or(&interfaces[0])
            .clone()
    };

    let link = Command::new("iw")
        .args(["dev", &device, "link"])
        .output()
        .context("failed to query Linux Wi-Fi link")?;
    if !link.status.success() {
        return Ok(unavailable(format!(
            "`iw dev {device} link` exited with status {}: {}",
            link.status,
            output_diagnostic(&link)
        )));
    }
    let text = String::from_utf8_lossy(&link.stdout);
    let ssid = linux_link_ssid(&text);
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

    let (channel, info_warning) = match Command::new("iw").args(["dev", &device, "info"]).output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            let channel = text.lines().find_map(|line| {
                let line = line.trim();
                let (_, tail) = line.split_once("channel ")?;
                tail.split_whitespace().next()?.parse::<u32>().ok()
            });
            (channel, None)
        }
        Ok(output) => (
            None,
            Some(format!(
                "`iw dev {device} info` exited with status {}: {}",
                output.status,
                output_diagnostic(&output)
            )),
        ),
        Err(error) => (
            None,
            Some(format!("failed to query `iw dev {device} info`: {error}")),
        ),
    };

    let mut detail = if ssid.is_some() {
        "Linux `iw` link diagnostics; PHY/link rate is not Internet throughput.".to_string()
    } else {
        format!("{device} is not associated with a Wi-Fi network")
    };
    if let Some(warning) = info_warning {
        detail.push_str(" Channel information was unavailable: ");
        detail.push_str(&warning);
    }

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
        detail,
    })
}

#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn linux_link_ssid(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("SSID:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn parse_iw_interfaces(text: &str) -> Vec<String> {
    let mut interfaces = Vec::new();
    let mut current = None;
    let mut current_is_managed = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(interface) = line.strip_prefix("Interface ") {
            if current_is_managed {
                if let Some(interface) = current.take() {
                    if !interfaces.iter().any(|known| known == &interface) {
                        interfaces.push(interface);
                    }
                }
            }
            let interface = interface.trim();
            current = (!interface.is_empty()).then(|| interface.to_string());
            current_is_managed = false;
        } else if current.is_some() {
            if let Some(interface_type) = line.strip_prefix("type ") {
                current_is_managed = interface_type.trim() == "managed";
            }
        }
    }
    if current_is_managed {
        if let Some(interface) = current {
            if !interfaces.iter().any(|known| known == &interface) {
                interfaces.push(interface);
            }
        }
    }
    interfaces
}

#[cfg(any(test, unix))]
fn dbm_to_percent(dbm: f64) -> f64 {
    ((dbm + 100.0) * 2.0).clamp(0.0, 100.0)
}

#[cfg(any(test, target_os = "windows", target_os = "macos"))]
fn band_from_channel(channel: u32) -> Option<String> {
    match channel {
        178..=233 => Some("6 GHz".to_string()),
        _ => None,
    }
}

#[cfg(any(test, target_os = "windows"))]
fn field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (left, right) = line.split_once(':')?;
        left.trim()
            .eq_ignore_ascii_case(key)
            .then(|| right.trim().to_string())
    })
}

#[cfg(target_os = "macos")]
fn colon_field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (left, right) = line.split_once(':')?;
        (left.trim() == key).then(|| right.trim().to_string())
    })
}

fn output_diagnostic(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
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
        assert_eq!(band_from_channel(6), None);
        assert_eq!(band_from_channel(44), None);
        assert_eq!(band_from_channel(201).as_deref(), Some("6 GHz"));
    }

    #[test]
    fn windows_blocks_ignore_preamble_and_missing_request_does_not_fallback() {
        let fixture = "There are 2 interfaces on the system:\n\n\
            Name                   : Wi-Fi\n\
            Description            : First adapter\n\
            State                  : disconnected\n\n\
            Name                   : Wi-Fi 2\n\
            Description            : USB adapter\n\
            SSID                   : Lab\n\
            Signal                 : 62%\n";

        let blocks = windows_interface_blocks(fixture);
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            field(
                &select_windows_interface_block(fixture, None)
                    .unwrap()
                    .unwrap(),
                "Name"
            )
            .as_deref(),
            Some("Wi-Fi 2")
        );
        assert_eq!(
            field(
                &select_windows_interface_block(fixture, Some("wi-fi 2"))
                    .unwrap()
                    .unwrap(),
                "Name"
            )
            .as_deref(),
            Some("Wi-Fi 2")
        );
        let error = select_windows_interface_block(fixture, Some("Missing")).unwrap_err();
        assert!(error.to_string().contains("was not found"));
    }

    #[test]
    fn parses_linux_iw_interface_inventory() {
        let fixture = "phy#0\n\
            Interface p2p-dev-wlan0\n\
                ifindex 4\n\
                type P2P-device\n\
            Interface wlan0\n\
                ifindex 3\n\
                type managed\n\
        phy#1\n\
            Interface monitor0\n\
                type monitor\n\
            Interface wlp4s0\n\
                type managed\n";
        assert_eq!(parse_iw_interfaces(fixture), vec!["wlan0", "wlp4s0"]);
    }

    #[test]
    fn parses_and_selects_macos_wifi_hardware_port_devices() {
        let fixture = "Hardware Port: Ethernet\n\
Device: en0\n\
Ethernet Address: 00:11:22:33:44:55\n\n\
Hardware Port: Wi-Fi\n\
Device: en7\n\
Ethernet Address: aa:bb:cc:dd:ee:ff\n\n\
Hardware Port: Thunderbolt Bridge\n\
Device: bridge0\n\n\
Hardware Port: AirPort\n\
Device: en8\n";
        let devices = parse_macos_wifi_devices(fixture);
        assert_eq!(devices, vec!["en7", "en8"]);
        assert_eq!(select_macos_wifi_device(&devices, None).unwrap(), "en7");
        assert_eq!(
            select_macos_wifi_device(&devices, Some("en8")).unwrap(),
            "en8"
        );
        let error = select_macos_wifi_device(&devices, Some("en0")).unwrap_err();
        assert!(error.to_string().contains("not a Wi-Fi hardware device"));

        let legacy = "Hardware Port: AirPort\nDevice: en1\n";
        assert_eq!(parse_macos_wifi_devices(legacy), vec!["en1"]);
        assert_eq!(
            macos_network_ssid("Current Wi-Fi Network: Lab").as_deref(),
            Some("Lab")
        );
        assert_eq!(
            macos_network_ssid("You are not associated with an AirPort network."),
            None
        );
    }
}
