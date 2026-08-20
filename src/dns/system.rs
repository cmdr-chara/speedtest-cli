use std::{
    fs,
    net::IpAddr,
    process::{Command, Output},
};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsConfigMode {
    Automatic,
    Manual,
    Unknown,
}

impl DnsConfigMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Automatic => "automatic / DHCP",
            Self::Manual => "manual",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsSystemState {
    pub interface: String,
    pub device: Option<String>,
    pub interface_index: Option<u32>,
    pub servers: Vec<IpAddr>,
    pub configured_servers: Vec<IpAddr>,
    pub mode: DnsConfigMode,
    pub backend: String,
    pub gateway: Option<IpAddr>,
    pub ipv6_default_route: bool,
}

impl DnsSystemState {
    pub fn can_configure(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            return self.interface_index.is_some();
        }
        #[cfg(target_os = "macos")]
        {
            return self.backend == "networksetup";
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            return self.backend == "NetworkManager";
        }
        #[allow(unreachable_code)]
        false
    }
}

pub fn inspect(interface_override: Option<&str>) -> Result<DnsSystemState> {
    #[cfg(target_os = "windows")]
    {
        return inspect_windows(interface_override);
    }
    #[cfg(target_os = "macos")]
    {
        return inspect_macos(interface_override);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return inspect_linux(interface_override);
    }
    #[allow(unreachable_code)]
    Err(anyhow!(
        "DNS system inspection is not supported on this platform"
    ))
}

pub fn apply_servers(state: &DnsSystemState, servers: &[IpAddr]) -> Result<()> {
    if servers.is_empty() {
        bail!("refusing to apply an empty DNS server list");
    }
    #[cfg(target_os = "windows")]
    {
        return apply_windows(state, servers);
    }
    #[cfg(target_os = "macos")]
    {
        return apply_macos(state, servers);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return apply_linux(state, servers);
    }
    #[allow(unreachable_code)]
    Err(anyhow!(
        "DNS configuration is not supported on this platform"
    ))
}

pub fn reset(state: &DnsSystemState) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        return reset_windows(state);
    }
    #[cfg(target_os = "macos")]
    {
        return reset_macos(state);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return reset_linux(state);
    }
    #[allow(unreachable_code)]
    Err(anyhow!("DNS reset is not supported on this platform"))
}

pub fn restore(state: &DnsSystemState) -> Result<()> {
    match state.mode {
        DnsConfigMode::Automatic => reset(state),
        DnsConfigMode::Manual => {
            let servers = if state.configured_servers.is_empty() {
                &state.servers
            } else {
                &state.configured_servers
            };
            apply_servers(state, servers)
        }
        DnsConfigMode::Unknown => {
            if state.configured_servers.is_empty() {
                bail!(
                    "the previous DNS mode was unknown; use `speedtest dns reset` to return to automatic DNS"
                );
            }
            apply_servers(state, &state.configured_servers)
        }
    }
}

pub fn flush_cache() {
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("ipconfig").arg("/flushdns").output();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("dscacheutil").arg("-flushcache").output();
        let _ = Command::new("killall")
            .args(["-HUP", "mDNSResponder"])
            .output();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if command_available("resolvectl") {
            let _ = Command::new("resolvectl").arg("flush-caches").output();
        } else if command_available("systemd-resolve") {
            let _ = Command::new("systemd-resolve")
                .arg("--flush-caches")
                .output();
        }
    }
}

#[cfg(target_os = "windows")]
fn inspect_windows(interface_override: Option<&str>) -> Result<DnsSystemState> {
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct WindowsState {
        interface_alias: String,
        interface_index: u32,
        servers: Vec<String>,
        automatic: bool,
        gateway: Option<String>,
        ipv6_default: bool,
    }

    let requested = interface_override.unwrap_or_default().replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference='Stop';\
         $requested='{requested}';\
         if ($requested) {{ $adapter=Get-NetAdapter -Name $requested -ErrorAction Stop; $idx=$adapter.ifIndex; $route=Get-NetRoute -DestinationPrefix '0.0.0.0/0' -ErrorAction SilentlyContinue | Where-Object InterfaceIndex -eq $idx | Sort-Object RouteMetric | Select-Object -First 1 }} else {{ $route=Get-NetRoute -DestinationPrefix '0.0.0.0/0' -ErrorAction Stop | Sort-Object RouteMetric | Select-Object -First 1; $idx=$route.InterfaceIndex; $adapter=Get-NetAdapter -InterfaceIndex $idx -ErrorAction Stop }};\
         $dns=@(Get-DnsClientServerAddress -InterfaceIndex $idx | ForEach-Object {{ $_.ServerAddresses }} | Where-Object {{ $_ }});\
         $automatic=$true;\
         try {{ $guid=$adapter.InterfaceGuid.ToString(); $key4=\"HKLM:\\SYSTEM\\CurrentControlSet\\Services\\Tcpip\\Parameters\\Interfaces\\{{$guid}}\"; $p4=Get-ItemProperty $key4 -ErrorAction SilentlyContinue; if ($p4 -and -not [string]::IsNullOrWhiteSpace([string]$p4.NameServer)) {{ $automatic=$false }}; $key6=\"HKLM:\\SYSTEM\\CurrentControlSet\\Services\\Tcpip6\\Parameters\\Interfaces\\{{$guid}}\"; $p6=Get-ItemProperty $key6 -ErrorAction SilentlyContinue; if ($p6 -and -not [string]::IsNullOrWhiteSpace([string]$p6.NameServer)) {{ $automatic=$false }} }} catch {{ }};\
         $v6=@(Get-NetRoute -AddressFamily IPv6 -DestinationPrefix '::/0' -ErrorAction SilentlyContinue).Count -gt 0;\
         [pscustomobject]@{{InterfaceAlias=$adapter.Name;InterfaceIndex=$idx;Servers=$dns;Automatic=$automatic;Gateway=if($route){{[string]$route.NextHop}}else{{$null}};IPv6Default=$v6}} | ConvertTo-Json -Compress"
    );
    let output = powershell(&script)?;
    let parsed: WindowsState = serde_json::from_slice(&output.stdout)
        .context("failed to parse Windows DNS configuration")?;
    let servers = parse_ip_list(parsed.servers.iter().map(String::as_str));

    Ok(DnsSystemState {
        interface: parsed.interface_alias,
        device: None,
        interface_index: Some(parsed.interface_index),
        configured_servers: if parsed.automatic {
            Vec::new()
        } else {
            servers.clone()
        },
        servers,
        mode: if parsed.automatic {
            DnsConfigMode::Automatic
        } else {
            DnsConfigMode::Manual
        },
        backend: "Windows DnsClient".to_string(),
        gateway: parsed.gateway.and_then(|gateway| gateway.parse().ok()),
        ipv6_default_route: parsed.ipv6_default,
    })
}

#[cfg(target_os = "windows")]
fn ensure_windows_admin() -> Result<()> {
    let script = "([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)";
    let output = powershell(script)?;
    if String::from_utf8_lossy(&output.stdout).trim() != "True" {
        bail!("administrator privileges are required to change DNS on Windows; reopen the terminal as Administrator");
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn apply_windows(state: &DnsSystemState, servers: &[IpAddr]) -> Result<()> {
    ensure_windows_admin()?;
    let index = state
        .interface_index
        .ok_or_else(|| anyhow!("Windows interface index is unavailable"))?;
    let values = servers
        .iter()
        .map(|server| format!("'{}'", server))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "$ErrorActionPreference='Stop'; Set-DnsClientServerAddress -InterfaceIndex {index} -ServerAddresses @({values})"
    );
    powershell(&script)?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn reset_windows(state: &DnsSystemState) -> Result<()> {
    ensure_windows_admin()?;
    let index = state
        .interface_index
        .ok_or_else(|| anyhow!("Windows interface index is unavailable"))?;
    powershell(&format!(
        "$ErrorActionPreference='Stop'; Set-DnsClientServerAddress -InterfaceIndex {index} -ResetServerAddresses"
    ))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn powershell(script: &str) -> Result<Output> {
    checked_output(
        Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", script]),
        "PowerShell DNS command",
    )
}

#[cfg(target_os = "macos")]
fn inspect_macos(interface_override: Option<&str>) -> Result<DnsSystemState> {
    let route = checked_output(
        Command::new("route").args(["-n", "get", "default"]),
        "macOS default route lookup",
    )?;
    let route_text = String::from_utf8_lossy(&route.stdout);
    let default_device = value_after_colon(&route_text, "interface")
        .ok_or_else(|| anyhow!("could not identify the active macOS network device"))?;
    let gateway = value_after_colon(&route_text, "gateway").and_then(|value| value.parse().ok());

    let order = checked_output(
        Command::new("networksetup").arg("-listnetworkserviceorder"),
        "macOS network service lookup",
    )?;
    let mappings = parse_macos_services(&String::from_utf8_lossy(&order.stdout));
    let (service, device) = if let Some(requested) = interface_override {
        mappings
            .iter()
            .find(|(service, device)| service == requested || device == requested)
            .cloned()
            .ok_or_else(|| anyhow!("macOS network service or device `{requested}` was not found"))?
    } else {
        mappings
            .iter()
            .find(|(_, device)| device == &default_device)
            .cloned()
            .ok_or_else(|| {
                anyhow!("could not map device {default_device} to a macOS network service")
            })?
    };

    let configured = checked_output(
        Command::new("networksetup").args(["-getdnsservers", &service]),
        "macOS DNS lookup",
    )?;
    let configured_text = String::from_utf8_lossy(&configured.stdout);
    let automatic = configured_text.contains("aren't any DNS Servers set");
    let configured_servers = if automatic {
        Vec::new()
    } else {
        parse_ip_list(configured_text.lines().map(str::trim))
    };
    let servers = if automatic {
        effective_macos_servers()?
    } else {
        configured_servers.clone()
    };
    let ipv6_default_route = Command::new("route")
        .args(["-n", "get", "-inet6", "default"])
        .output()
        .is_ok_and(|output| output.status.success());

    Ok(DnsSystemState {
        interface: service,
        device: Some(device),
        interface_index: None,
        servers,
        configured_servers,
        mode: if automatic {
            DnsConfigMode::Automatic
        } else {
            DnsConfigMode::Manual
        },
        backend: "networksetup".to_string(),
        gateway,
        ipv6_default_route,
    })
}

#[cfg(target_os = "macos")]
fn effective_macos_servers() -> Result<Vec<IpAddr>> {
    let output = checked_output(Command::new("scutil").arg("--dns"), "macOS resolver lookup")?;
    let text = String::from_utf8_lossy(&output.stdout);
    let servers = text
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            line.strip_prefix("nameserver[")
                .and_then(|rest| rest.split_once(':').map(|(_, value)| value.trim()))
                .and_then(|value| value.parse::<IpAddr>().ok())
        })
        .collect::<Vec<_>>();
    Ok(dedup_ips(servers))
}

#[cfg(target_os = "macos")]
fn parse_macos_services(text: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut pending_service: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('(') && line.contains(')') && !line.starts_with("(Hardware Port") {
            if let Some((_, service)) = line.split_once(')') {
                pending_service = Some(service.trim().trim_start_matches('*').trim().to_string());
            }
        } else if line.contains("Device:") {
            let device = line
                .split("Device:")
                .nth(1)
                .and_then(|value| value.split(')').next())
                .map(str::trim)
                .unwrap_or_default();
            if let Some(service) = pending_service.take() {
                if !service.is_empty() && !device.is_empty() {
                    result.push((service, device.to_string()));
                }
            }
        }
    }
    result
}

#[cfg(target_os = "macos")]
fn ensure_macos_root() -> Result<()> {
    let output = checked_output(Command::new("id").arg("-u"), "macOS privilege check")?;
    if String::from_utf8_lossy(&output.stdout).trim() != "0" {
        bail!("administrator privileges are required to change DNS on macOS; run the command with sudo");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn apply_macos(state: &DnsSystemState, servers: &[IpAddr]) -> Result<()> {
    ensure_macos_root()?;
    let mut command = Command::new("networksetup");
    command.arg("-setdnsservers").arg(&state.interface);
    for server in servers {
        command.arg(server.to_string());
    }
    checked_output(&mut command, "macOS DNS configuration")?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn reset_macos(state: &DnsSystemState) -> Result<()> {
    ensure_macos_root()?;
    checked_output(
        Command::new("networksetup").args(["-setdnsservers", &state.interface, "empty"]),
        "macOS DNS reset",
    )?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn inspect_linux(interface_override: Option<&str>) -> Result<DnsSystemState> {
    let route = checked_output(
        Command::new("ip").args(["route", "show", "default"]),
        "Linux default route lookup",
    )?;
    let route_text = String::from_utf8_lossy(&route.stdout);
    let route_line = route_text
        .lines()
        .next()
        .ok_or_else(|| anyhow!("no IPv4 default route was found"))?;
    let default_device = token_after(route_line, "dev")
        .ok_or_else(|| anyhow!("could not identify the active Linux interface"))?;
    let device = interface_override.unwrap_or(&default_device).to_string();
    let gateway = token_after(route_line, "via").and_then(|value| value.parse().ok());
    let ipv6_default_route = Command::new("ip")
        .args(["-6", "route", "show", "default"])
        .output()
        .is_ok_and(|output| output.status.success() && !output.stdout.is_empty());

    let mut configured_servers = Vec::new();
    let mut mode = DnsConfigMode::Unknown;
    let mut backend = "read-only resolver inspection".to_string();
    let mut connection_name = None;

    if command_available("nmcli") {
        if let Ok(output) = checked_output(
            Command::new("nmcli").args(["-g", "GENERAL.CONNECTION", "device", "show", &device]),
            "NetworkManager connection lookup",
        ) {
            let connection = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !connection.is_empty() && connection != "--" {
                configured_servers.extend(nmcli_ips(&connection, "ipv4.dns"));
                configured_servers.extend(nmcli_ips(&connection, "ipv6.dns"));
                let ignore4 = nmcli_value(&connection, "ipv4.ignore-auto-dns").unwrap_or_default();
                let ignore6 = nmcli_value(&connection, "ipv6.ignore-auto-dns").unwrap_or_default();
                mode = if configured_servers.is_empty() && !is_true(&ignore4) && !is_true(&ignore6)
                {
                    DnsConfigMode::Automatic
                } else {
                    DnsConfigMode::Manual
                };
                backend = "NetworkManager".to_string();
                connection_name = Some(connection);
            }
        }
    }

    let mut servers = effective_linux_servers(&device);
    if servers.is_empty() {
        servers = configured_servers.clone();
    }

    Ok(DnsSystemState {
        interface: device.clone(),
        device: connection_name,
        interface_index: None,
        servers,
        configured_servers,
        mode,
        backend,
        gateway,
        ipv6_default_route,
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn effective_linux_servers(device: &str) -> Vec<IpAddr> {
    if command_available("resolvectl") {
        if let Ok(output) = Command::new("resolvectl").args(["dns", device]).output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                let servers = text
                    .split_whitespace()
                    .filter_map(|token| token.trim_matches(':').parse::<IpAddr>().ok())
                    .collect::<Vec<_>>();
                if !servers.is_empty() {
                    return dedup_ips(servers);
                }
            }
        }
    }

    fs::read_to_string("/etc/resolv.conf")
        .ok()
        .map(|content| {
            content
                .lines()
                .filter_map(|line| {
                    let line = line.trim();
                    line.strip_prefix("nameserver")
                        .and_then(|value| value.split_whitespace().next())
                        .and_then(|value| value.parse().ok())
                })
                .collect::<Vec<IpAddr>>()
        })
        .map(dedup_ips)
        .unwrap_or_default()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn nmcli_ips(connection: &str, field: &str) -> Vec<IpAddr> {
    nmcli_value(connection, field)
        .map(|value| {
            value
                .split([',', ' ', ';'])
                .filter_map(|item| item.trim().parse::<IpAddr>().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn nmcli_value(connection: &str, field: &str) -> Option<String> {
    let output = Command::new("nmcli")
        .args(["-g", field, "connection", "show", connection])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn apply_linux(state: &DnsSystemState, servers: &[IpAddr]) -> Result<()> {
    if state.backend != "NetworkManager" {
        bail!(
            "automatic DNS configuration on Linux currently requires NetworkManager; resolver inspection and benchmarking still work"
        );
    }
    let connection = state
        .device
        .as_deref()
        .ok_or_else(|| anyhow!("NetworkManager connection name is unavailable"))?;
    let ipv4 = servers
        .iter()
        .filter(|server| server.is_ipv4())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let ipv6 = servers
        .iter()
        .filter(|server| server.is_ipv6())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");

    checked_output(
        Command::new("nmcli").args([
            "connection",
            "modify",
            connection,
            "ipv4.ignore-auto-dns",
            "yes",
            "ipv4.dns",
            &ipv4,
            "ipv6.ignore-auto-dns",
            "yes",
            "ipv6.dns",
            &ipv6,
        ]),
        "NetworkManager DNS configuration",
    )?;
    checked_output(
        Command::new("nmcli").args(["connection", "up", connection]),
        "NetworkManager connection reload",
    )?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn reset_linux(state: &DnsSystemState) -> Result<()> {
    if state.backend != "NetworkManager" {
        bail!(
            "automatic DNS reset on Linux currently requires NetworkManager; restore your resolver with your network manager"
        );
    }
    let connection = state
        .device
        .as_deref()
        .ok_or_else(|| anyhow!("NetworkManager connection name is unavailable"))?;
    checked_output(
        Command::new("nmcli").args([
            "connection",
            "modify",
            connection,
            "ipv4.ignore-auto-dns",
            "no",
            "ipv4.dns",
            "",
            "ipv6.ignore-auto-dns",
            "no",
            "ipv6.dns",
            "",
        ]),
        "NetworkManager DNS reset",
    )?;
    checked_output(
        Command::new("nmcli").args(["connection", "up", connection]),
        "NetworkManager connection reload",
    )?;
    Ok(())
}

fn checked_output(command: &mut Command, description: &str) -> Result<Output> {
    let output = command
        .output()
        .with_context(|| format!("failed to run {description}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        bail!("{description} failed: {detail}");
    }
    Ok(output)
}

fn parse_ip_list<'a>(values: impl Iterator<Item = &'a str>) -> Vec<IpAddr> {
    dedup_ips(
        values
            .filter_map(|value| value.trim().parse::<IpAddr>().ok())
            .collect(),
    )
}

fn dedup_ips(mut values: Vec<IpAddr>) -> Vec<IpAddr> {
    values.sort_unstable();
    values.dedup();
    values
}

fn command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn value_after_colon(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (left, right) = line.split_once(':')?;
        (left.trim() == key).then(|| right.trim().to_string())
    })
}

fn token_after(text: &str, key: &str) -> Option<String> {
    let tokens = text.split_whitespace().collect::<Vec<_>>();
    tokens
        .windows(2)
        .find_map(|pair| (pair[0] == key).then(|| pair[1].to_string()))
}

fn is_true(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "yes" | "true" | "1"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_route_tokens() {
        let route = "default via 192.168.1.1 dev eth0 proto dhcp";
        assert_eq!(token_after(route, "dev").as_deref(), Some("eth0"));
        assert_eq!(token_after(route, "via").as_deref(), Some("192.168.1.1"));
    }

    #[test]
    fn parses_colon_values() {
        let route = "   gateway: 192.168.1.1\n interface: en0\n";
        assert_eq!(
            value_after_colon(route, "interface").as_deref(),
            Some("en0")
        );
    }

    #[test]
    fn deduplicates_ip_addresses() {
        let values = vec![
            "1.1.1.1".parse().unwrap(),
            "1.0.0.1".parse().unwrap(),
            "1.1.1.1".parse().unwrap(),
        ];
        assert_eq!(dedup_ips(values).len(), 2);
    }
}
