use std::{
    net::IpAddr,
    process::{Command, Output},
};

#[cfg(all(unix, not(target_os = "macos")))]
use std::fs;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsSystemState {
    pub interface: String,
    pub device: Option<String>,
    pub interface_index: Option<u32>,
    pub servers: Vec<IpAddr>,
    pub configured_servers: Vec<IpAddr>,
    pub mode: DnsConfigMode,
    pub backend: String,
    pub gateway: Option<IpAddr>,
    /// Zone identifier required to reach an IPv6 link-local gateway.
    ///
    /// This is normally an interface index on Windows and an interface name
    /// on Unix. It is additive so older rollback snapshots remain readable.
    #[serde(default)]
    pub gateway_scope: Option<String>,
    #[serde(default)]
    pub ipv4_default_route: bool,
    pub ipv6_default_route: bool,
    /// The original NetworkManager per-family `ignore-auto-dns` values.
    ///
    /// These are additive so DNS snapshots written before this state was
    /// captured remain readable. `None` also keeps non-NetworkManager
    /// platforms on their existing restore path.
    #[serde(default)]
    pub ipv4_ignore_auto_dns: Option<bool>,
    #[serde(default)]
    pub ipv6_ignore_auto_dns: Option<bool>,
    /// Per-family automatic/manual state where the platform exposes it.
    #[serde(default)]
    pub ipv4_automatic: Option<bool>,
    #[serde(default)]
    pub ipv6_automatic: Option<bool>,
}

impl DnsSystemState {
    pub fn can_configure(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            let restorable_family_modes = match (self.ipv4_automatic, self.ipv6_automatic) {
                (Some(ipv4), Some(ipv6)) => ipv4 == ipv6,
                _ => false,
            };
            return self.interface_index.is_some() && restorable_family_modes;
        }
        #[cfg(target_os = "macos")]
        {
            return self.backend == "networksetup"
                && macos_state_is_configurable(self.mode, &self.configured_servers);
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            return self.backend == "NetworkManager"
                && self.ipv4_ignore_auto_dns.is_some()
                && self.ipv6_ignore_auto_dns.is_some();
        }
        #[allow(unreachable_code)]
        false
    }
}

#[cfg(any(target_os = "macos", test))]
fn macos_state_is_configurable(mode: DnsConfigMode, configured_servers: &[IpAddr]) -> bool {
    match mode {
        DnsConfigMode::Automatic => true,
        DnsConfigMode::Manual => !configured_servers.is_empty(),
        DnsConfigMode::Unknown => false,
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
    if !state.can_configure() {
        bail!("{} cannot safely preserve and restore DNS", state.backend);
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
    if !state.can_configure() {
        bail!("{} cannot safely preserve and restore DNS", state.backend);
    }
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
    #[cfg(all(unix, not(target_os = "macos")))]
    if state.backend == "NetworkManager"
        && state.ipv4_ignore_auto_dns.is_some()
        && state.ipv6_ignore_auto_dns.is_some()
    {
        return restore_linux_snapshot(state);
    }

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

#[cfg(any(target_os = "windows", test))]
fn windows_dns_inspection_script(interface_override: Option<&str>) -> String {
    let requested = interface_override.unwrap_or_default().replace('\'', "''");
    format!(
        "$ErrorActionPreference='Stop';\
         function Get-EffectiveRouteMetric($route) {{ $ipInterface=Get-NetIPInterface -InterfaceIndex $route.InterfaceIndex -AddressFamily $route.AddressFamily -ErrorAction SilentlyContinue | Select-Object -First 1; if ($ipInterface) {{ return [int]$route.RouteMetric + [int]$ipInterface.InterfaceMetric }}; return [int]$route.RouteMetric }};\
         $requested='{requested}';\
         if ($requested) {{ $adapter=Get-NetAdapter -Name $requested -ErrorAction Stop; $idx=$adapter.ifIndex }} else {{ $selectedRoute=Get-NetRoute -AddressFamily IPv4 -DestinationPrefix '0.0.0.0/0' -ErrorAction SilentlyContinue | Sort-Object @{{Expression={{Get-EffectiveRouteMetric $_}}}} | Select-Object -First 1; if (-not $selectedRoute) {{ $selectedRoute=Get-NetRoute -AddressFamily IPv6 -DestinationPrefix '::/0' -ErrorAction SilentlyContinue | Sort-Object @{{Expression={{Get-EffectiveRouteMetric $_}}}} | Select-Object -First 1 }}; if (-not $selectedRoute) {{ throw 'No IPv4 or IPv6 default route was found' }}; $idx=$selectedRoute.InterfaceIndex; $adapter=Get-NetAdapter -InterfaceIndex $idx -ErrorAction Stop }};\
         $route4=Get-NetRoute -AddressFamily IPv4 -DestinationPrefix '0.0.0.0/0' -ErrorAction SilentlyContinue | Where-Object InterfaceIndex -eq $idx | Sort-Object @{{Expression={{Get-EffectiveRouteMetric $_}}}} | Select-Object -First 1;\
         $route6=Get-NetRoute -AddressFamily IPv6 -DestinationPrefix '::/0' -ErrorAction SilentlyContinue | Where-Object InterfaceIndex -eq $idx | Sort-Object @{{Expression={{Get-EffectiveRouteMetric $_}}}} | Select-Object -First 1;\
         $dns=@(Get-DnsClientServerAddress -InterfaceIndex $idx | ForEach-Object {{ $_.ServerAddresses }} | Where-Object {{ $_ }});\
         $automatic4=$true;$automatic6=$true;\
         try {{ $guid=$adapter.InterfaceGuid.ToString(); $key4=\"HKLM:\\SYSTEM\\CurrentControlSet\\Services\\Tcpip\\Parameters\\Interfaces\\{{$guid}}\"; $p4=Get-ItemProperty $key4 -ErrorAction SilentlyContinue; if ($p4 -and -not [string]::IsNullOrWhiteSpace([string]$p4.NameServer)) {{ $automatic4=$false }}; $key6=\"HKLM:\\SYSTEM\\CurrentControlSet\\Services\\Tcpip6\\Parameters\\Interfaces\\{{$guid}}\"; $p6=Get-ItemProperty $key6 -ErrorAction SilentlyContinue; if ($p6 -and -not [string]::IsNullOrWhiteSpace([string]$p6.NameServer)) {{ $automatic6=$false }} }} catch {{ }};\
         $automatic=$automatic4 -and $automatic6;\
         [pscustomobject]@{{InterfaceAlias=$adapter.Name;InterfaceIndex=$idx;Servers=$dns;Automatic=$automatic;IPv4Automatic=$automatic4;IPv6Automatic=$automatic6;Gateway=if($route4){{[string]$route4.NextHop}} elseif($route6){{[string]$route6.NextHop}} else{{$null}};IPv4Default=[bool]$route4;IPv6Default=[bool]$route6}} | ConvertTo-Json -Compress"
    )
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
        #[serde(rename = "IPv4Automatic")]
        ipv4_automatic: bool,
        #[serde(rename = "IPv6Automatic")]
        ipv6_automatic: bool,
        gateway: Option<String>,
        #[serde(rename = "IPv4Default")]
        ipv4_default: bool,
        #[serde(rename = "IPv6Default")]
        ipv6_default: bool,
    }

    let script = windows_dns_inspection_script(interface_override);
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
        gateway: parsed.gateway.as_deref().and_then(parse_gateway_ip),
        gateway_scope: parsed
            .gateway
            .as_deref()
            .and_then(parse_gateway_ip)
            .filter(is_ipv6_link_local)
            .map(|_| parsed.interface_index.to_string()),
        ipv4_default_route: parsed.ipv4_default,
        ipv6_default_route: parsed.ipv6_default,
        ipv4_ignore_auto_dns: None,
        ipv6_ignore_auto_dns: None,
        ipv4_automatic: Some(parsed.ipv4_automatic),
        ipv6_automatic: Some(parsed.ipv6_automatic),
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

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct MacosDefaultRoute {
    device: String,
    gateway: Option<IpAddr>,
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_default_route(text: &str) -> Option<MacosDefaultRoute> {
    Some(MacosDefaultRoute {
        device: value_after_colon(text, "interface")?,
        gateway: value_after_colon(text, "gateway").and_then(|value| parse_gateway_ip(&value)),
    })
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_dns_configuration(text: &str) -> (DnsConfigMode, Vec<IpAddr>) {
    let configured_servers = parse_ip_list(text.lines().map(str::trim));
    if !configured_servers.is_empty() {
        return (DnsConfigMode::Manual, configured_servers);
    }

    let normalized = text.to_ascii_lowercase();
    if normalized.contains("aren't any dns servers set")
        || normalized.contains("there are no dns servers set")
    {
        (DnsConfigMode::Automatic, Vec::new())
    } else {
        // `networksetup` localizes this sentence on some systems. An empty,
        // unrecognized response is not enough evidence that resetting DNS can
        // reproduce the original state, so keep the backend read-only.
        (DnsConfigMode::Unknown, Vec::new())
    }
}

#[cfg(any(target_os = "macos", test))]
fn select_macos_route_state(
    device: &str,
    ipv4_route: Option<&MacosDefaultRoute>,
    ipv6_route: Option<&MacosDefaultRoute>,
) -> (Option<IpAddr>, bool, bool) {
    let selected_ipv4 = ipv4_route.filter(|route| route.device == device);
    let selected_ipv6 = ipv6_route.filter(|route| route.device == device);
    let gateway = selected_ipv4
        .and_then(|route| route.gateway)
        .or_else(|| selected_ipv6.and_then(|route| route.gateway));
    (gateway, selected_ipv4.is_some(), selected_ipv6.is_some())
}

#[cfg(target_os = "macos")]
fn lookup_macos_default_route(ipv6: bool) -> Result<Option<MacosDefaultRoute>> {
    let mut command = Command::new("route");
    command.args(["-n", "get"]);
    if ipv6 {
        command.arg("-inet6");
    }
    command.arg("default");
    let family = if ipv6 { "IPv6" } else { "IPv4" };
    let output = command
        .output()
        .with_context(|| format!("failed to run macOS {family} default route lookup"))?;
    if !output.status.success() {
        return Ok(None);
    }
    parse_macos_default_route(&String::from_utf8_lossy(&output.stdout))
        .map(Some)
        .ok_or_else(|| anyhow!("could not parse the macOS {family} default route"))
}

#[cfg(target_os = "macos")]
fn inspect_macos(interface_override: Option<&str>) -> Result<DnsSystemState> {
    let ipv4_route = lookup_macos_default_route(false)?;
    let ipv6_route = lookup_macos_default_route(true)?;
    let default_device = ipv4_route
        .as_ref()
        .or(ipv6_route.as_ref())
        .map(|route| route.device.as_str());

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
        let default_device = default_device
            .ok_or_else(|| anyhow!("no IPv4 or IPv6 default route was found on macOS"))?;
        mappings
            .iter()
            .find(|(_, device)| device == default_device)
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
    let (mode, configured_servers) = parse_macos_dns_configuration(&configured_text);
    let servers = if mode == DnsConfigMode::Manual {
        configured_servers.clone()
    } else {
        effective_macos_servers()?
    };
    let (gateway, ipv4_default_route, ipv6_default_route) =
        select_macos_route_state(&device, ipv4_route.as_ref(), ipv6_route.as_ref());

    Ok(DnsSystemState {
        interface: service,
        device: Some(device.clone()),
        interface_index: None,
        servers,
        configured_servers,
        mode,
        backend: "networksetup".to_string(),
        gateway,
        gateway_scope: gateway
            .filter(is_ipv6_link_local)
            .map(|_| device.clone()),
        ipv4_default_route,
        ipv6_default_route,
        ipv4_ignore_auto_dns: None,
        ipv6_ignore_auto_dns: None,
        ipv4_automatic: None,
        ipv6_automatic: None,
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
#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxDefaultRoute {
    device: String,
    gateway: Option<IpAddr>,
}

#[cfg(all(unix, not(target_os = "macos")))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxRouteSelection {
    device: String,
    ipv4: Option<LinuxDefaultRoute>,
    ipv6: Option<LinuxDefaultRoute>,
}

#[cfg(all(unix, not(target_os = "macos")))]
fn parse_linux_default_routes(text: &str) -> Vec<LinuxDefaultRoute> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.split_whitespace().next() != Some("default") {
                return None;
            }
            let device = token_after(line, "dev")?;
            let gateway = token_after(line, "via").and_then(|value| parse_gateway_ip(&value));
            Some(LinuxDefaultRoute { device, gateway })
        })
        .collect()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn select_linux_routes(
    ipv4_text: &str,
    ipv6_text: &str,
    interface_override: Option<&str>,
) -> Result<LinuxRouteSelection> {
    let ipv4_routes = parse_linux_default_routes(ipv4_text);
    let ipv6_routes = parse_linux_default_routes(ipv6_text);
    let device = if let Some(requested) = interface_override {
        requested.to_string()
    } else {
        ipv4_routes
            .first()
            .or_else(|| ipv6_routes.first())
            .map(|route| route.device.clone())
            .ok_or_else(|| anyhow!("no IPv4 or IPv6 default route was found"))?
    };

    let ipv4 = ipv4_routes.into_iter().find(|route| route.device == device);
    let ipv6 = ipv6_routes.into_iter().find(|route| route.device == device);
    Ok(LinuxRouteSelection { device, ipv4, ipv6 })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn allow_global_linux_resolver_fallback(
    interface_override: Option<&str>,
    ipv4_default_route: bool,
    ipv6_default_route: bool,
) -> bool {
    interface_override.is_none() || ipv4_default_route || ipv6_default_route
}

#[cfg(all(unix, not(target_os = "macos")))]
fn inspect_linux(interface_override: Option<&str>) -> Result<DnsSystemState> {
    if let Some(requested) = interface_override {
        if requested.is_empty() {
            bail!("Linux interface override cannot be empty");
        }
        checked_output(
            Command::new("ip").args(["link", "show", "dev", requested]),
            &format!("Linux interface `{requested}` lookup"),
        )?;
    }

    let ipv4_routes = checked_output(
        Command::new("ip").args(["-4", "route", "show", "default"]),
        "Linux IPv4 default route lookup",
    )?;
    let ipv6_routes = checked_output(
        Command::new("ip").args(["-6", "route", "show", "default"]),
        "Linux IPv6 default route lookup",
    )?;
    let routes = select_linux_routes(
        &String::from_utf8_lossy(&ipv4_routes.stdout),
        &String::from_utf8_lossy(&ipv6_routes.stdout),
        interface_override,
    )?;
    let device = routes.device;
    let gateway = routes
        .ipv4
        .as_ref()
        .and_then(|route| route.gateway)
        .or_else(|| routes.ipv6.as_ref().and_then(|route| route.gateway));
    let ipv4_default_route = routes.ipv4.is_some();
    let ipv6_default_route = routes.ipv6.is_some();

    let mut configured_servers = Vec::new();
    let mut mode = DnsConfigMode::Unknown;
    let mut backend = "read-only resolver inspection".to_string();
    let mut connection_name = None;
    let mut ipv4_ignore_auto_dns = None;
    let mut ipv6_ignore_auto_dns = None;

    if command_available("nmcli") {
        if let Ok(output) = checked_output(
            Command::new("nmcli").args([
                "--escape",
                "no",
                "-g",
                "GENERAL.CONNECTION",
                "device",
                "show",
                &device,
            ]),
            "NetworkManager connection lookup",
        ) {
            let connection = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !connection.is_empty() && connection != "--" {
                let configured4 = nmcli_ips(&connection, "ipv4.dns");
                let configured6 = nmcli_ips(&connection, "ipv6.dns");
                let ignore4 = nmcli_value(&connection, "ipv4.ignore-auto-dns")
                    .as_deref()
                    .and_then(parse_nmcli_bool);
                let ignore6 = nmcli_value(&connection, "ipv6.ignore-auto-dns")
                    .as_deref()
                    .and_then(parse_nmcli_bool);
                let configured_dns_known = configured4.is_some() && configured6.is_some();
                if let Some(servers) = &configured4 {
                    configured_servers.extend(servers);
                }
                if let Some(servers) = &configured6 {
                    configured_servers.extend(servers);
                }
                mode = if configured_dns_known
                    && configured_servers.is_empty()
                    && ignore4 == Some(false)
                    && ignore6 == Some(false)
                {
                    DnsConfigMode::Automatic
                } else if !configured_servers.is_empty()
                    || ignore4 == Some(true)
                    || ignore6 == Some(true)
                {
                    DnsConfigMode::Manual
                } else {
                    DnsConfigMode::Unknown
                };
                // Only advertise exact per-family restore data when both the
                // setting and its associated server list were read safely.
                ipv4_ignore_auto_dns = configured4.is_some().then_some(ignore4).flatten();
                ipv6_ignore_auto_dns = configured6.is_some().then_some(ignore6).flatten();
                backend = "NetworkManager".to_string();
                connection_name = Some(connection);
            }
        }
    }

    let allow_global_resolver_fallback = allow_global_linux_resolver_fallback(
        interface_override,
        ipv4_default_route,
        ipv6_default_route,
    );
    let mut servers = effective_linux_servers(&device, allow_global_resolver_fallback);
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
        gateway_scope: gateway
            .filter(is_ipv6_link_local)
            .map(|_| device.clone()),
        ipv4_default_route,
        ipv6_default_route,
        ipv4_ignore_auto_dns,
        ipv6_ignore_auto_dns,
        ipv4_automatic: None,
        ipv6_automatic: None,
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn effective_linux_servers(device: &str, allow_global_fallback: bool) -> Vec<IpAddr> {
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

    if !allow_global_fallback {
        return Vec::new();
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
fn nmcli_ips(connection: &str, field: &str) -> Option<Vec<IpAddr>> {
    let value = nmcli_value(connection, field)?;
    parse_nmcli_ips(&value)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn parse_nmcli_ips(value: &str) -> Option<Vec<IpAddr>> {
    value
        .split(|character: char| character == ',' || character == ';' || character.is_whitespace())
        .filter(|item| !item.is_empty())
        .map(|item| item.replace("\\:", ":").parse::<IpAddr>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn nmcli_value(connection: &str, field: &str) -> Option<String> {
    let output = Command::new("nmcli")
        .args([
            "--escape",
            "no",
            "-g",
            field,
            "connection",
            "show",
            connection,
        ])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_connection<'a>(state: &'a DnsSystemState, operation: &str) -> Result<&'a str> {
    if state.backend != "NetworkManager" {
        bail!(
            "{operation} on Linux currently requires NetworkManager; resolver inspection and benchmarking still work"
        );
    }
    state
        .device
        .as_deref()
        .ok_or_else(|| anyhow!("NetworkManager connection name is unavailable"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn configure_linux_dns(
    state: &DnsSystemState,
    ipv4: &str,
    ipv6: &str,
    ignore_ipv4_auto_dns: bool,
    ignore_ipv6_auto_dns: bool,
    description: &str,
) -> Result<()> {
    let connection = linux_connection(state, description)?;
    let ignore4 = if ignore_ipv4_auto_dns { "yes" } else { "no" };
    let ignore6 = if ignore_ipv6_auto_dns { "yes" } else { "no" };
    checked_output(
        Command::new("nmcli").args([
            "connection",
            "modify",
            connection,
            "ipv4.ignore-auto-dns",
            ignore4,
            "ipv4.dns",
            ipv4,
            "ipv6.ignore-auto-dns",
            ignore6,
            "ipv6.dns",
            ipv6,
        ]),
        description,
    )?;
    checked_output(
        Command::new("nmcli").args(["connection", "up", connection]),
        "NetworkManager connection reload",
    )?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn apply_linux(state: &DnsSystemState, servers: &[IpAddr]) -> Result<()> {
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

    configure_linux_dns(
        state,
        &ipv4,
        &ipv6,
        true,
        true,
        "NetworkManager DNS configuration",
    )
}

#[cfg(all(unix, not(target_os = "macos")))]
fn reset_linux(state: &DnsSystemState) -> Result<()> {
    configure_linux_dns(state, "", "", false, false, "NetworkManager DNS reset")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn restore_linux_snapshot(state: &DnsSystemState) -> Result<()> {
    let ipv4 = state
        .configured_servers
        .iter()
        .filter(|server| server.is_ipv4())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let ipv6 = state
        .configured_servers
        .iter()
        .filter(|server| server.is_ipv6())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    configure_linux_dns(
        state,
        &ipv4,
        &ipv6,
        state.ipv4_ignore_auto_dns.unwrap_or(false),
        state.ipv6_ignore_auto_dns.unwrap_or(false),
        "NetworkManager DNS restoration",
    )
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

#[cfg(any(target_os = "windows", target_os = "macos", test))]
fn parse_ip_list<'a>(values: impl Iterator<Item = &'a str>) -> Vec<IpAddr> {
    dedup_ips(
        values
            .filter_map(|value| value.trim().parse::<IpAddr>().ok())
            .collect(),
    )
}

fn dedup_ips(values: Vec<IpAddr>) -> Vec<IpAddr> {
    let mut unique = Vec::with_capacity(values.len());
    for value in values {
        if !unique.contains(&value) {
            unique.push(value);
        }
    }
    unique
}

fn parse_gateway_ip(value: &str) -> Option<IpAddr> {
    let value = value.trim();
    value.parse().ok().or_else(|| {
        let (address, scope) = value.rsplit_once('%')?;
        (!scope.is_empty())
            .then(|| address.parse::<IpAddr>().ok())
            .flatten()
    })
}

fn is_ipv6_link_local(address: &IpAddr) -> bool {
    matches!(address, IpAddr::V6(address) if address.is_unicast_link_local())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(any(target_os = "macos", test))]
fn value_after_colon(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (left, right) = line.split_once(':')?;
        (left.trim() == key).then(|| right.trim().to_string())
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn token_after(text: &str, key: &str) -> Option<String> {
    let tokens = text.split_whitespace().collect::<Vec<_>>();
    tokens
        .windows(2)
        .find_map(|pair| (pair[0] == key).then(|| pair[1].to_string()))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn parse_nmcli_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn parses_route_tokens() {
        let route = "default via 192.168.1.1 dev eth0 proto dhcp";
        assert_eq!(token_after(route, "dev").as_deref(), Some("eth0"));
        assert_eq!(token_after(route, "via").as_deref(), Some("192.168.1.1"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn parses_linux_default_routes_and_skips_non_defaults() {
        let routes = parse_linux_default_routes(
            "192.0.2.0/24 dev eth0 scope link\n\
             default via 192.0.2.1 dev eth0 proto dhcp metric 100\n\
             default dev wg0 metric 200\n",
        );
        assert_eq!(
            routes,
            vec![
                LinuxDefaultRoute {
                    device: "eth0".to_string(),
                    gateway: Some("192.0.2.1".parse().unwrap()),
                },
                LinuxDefaultRoute {
                    device: "wg0".to_string(),
                    gateway: None,
                },
            ]
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn selects_ipv6_default_when_ipv4_is_absent() {
        let selection = select_linux_routes(
            "",
            "default via fe80::1 dev enp0s3 proto ra metric 100\n",
            None,
        )
        .unwrap();
        assert_eq!(selection.device, "enp0s3");
        assert!(selection.ipv4.is_none());
        assert_eq!(
            selection.ipv6.and_then(|route| route.gateway),
            Some("fe80::1".parse().unwrap())
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn route_state_is_scoped_to_requested_interface() {
        let selection = select_linux_routes(
            "default via 192.0.2.1 dev eth0 metric 100\n",
            "default via fe80::1 dev wlan0 metric 200\n",
            Some("wlan0"),
        )
        .unwrap();
        assert_eq!(selection.device, "wlan0");
        assert!(selection.ipv4.is_none());
        assert!(selection.ipv6.is_some());

        let non_default = select_linux_routes(
            "default via 192.0.2.1 dev eth0 metric 100\n",
            "default via fe80::1 dev wlan0 metric 200\n",
            Some("dummy0"),
        )
        .unwrap();
        assert!(non_default.ipv4.is_none());
        assert!(non_default.ipv6.is_none());
        assert!(!allow_global_linux_resolver_fallback(
            Some("dummy0"),
            false,
            false
        ));
        assert!(allow_global_linux_resolver_fallback(
            Some("wlan0"),
            false,
            true
        ));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn default_selection_prefers_ipv4_when_both_families_exist() {
        let selection = select_linux_routes(
            "default via 192.0.2.1 dev eth0 metric 100\n",
            "default via fe80::1 dev wlan0 metric 100\n",
            None,
        )
        .unwrap();
        assert_eq!(selection.device, "eth0");
        assert!(selection.ipv4.is_some());
        assert!(selection.ipv6.is_none());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn rejects_default_selection_without_any_default_route() {
        let error = select_linux_routes("", "", None).unwrap_err();
        assert!(error.to_string().contains("no IPv4 or IPv6 default route"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn parses_network_manager_booleans() {
        assert_eq!(parse_nmcli_bool("yes"), Some(true));
        assert_eq!(parse_nmcli_bool("FALSE"), Some(false));
        assert_eq!(parse_nmcli_bool("unknown"), None);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn parses_network_manager_ip_lists_without_discarding_invalid_state() {
        assert_eq!(
            parse_nmcli_ips("1.1.1.1, 1.0.0.1\n2606:4700:4700::1111")
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            parse_nmcli_ips(r"2606\:4700\:4700\:\:1111"),
            Some(vec!["2606:4700:4700::1111".parse().unwrap()])
        );
        assert_eq!(parse_nmcli_ips(""), Some(Vec::new()));
        assert_eq!(parse_nmcli_ips("1.1.1.1,not-an-address"), None);
    }

    #[test]
    fn parses_macos_route_values() {
        let text = "   route to: default\n   gateway: 192.168.1.1\n interface: en0\n";
        assert_eq!(
            parse_macos_default_route(text),
            Some(MacosDefaultRoute {
                device: "en0".to_string(),
                gateway: Some("192.168.1.1".parse().unwrap()),
            })
        );

        let scoped = "route to: default\ngateway: fe80::1%en0\ninterface: en0\n";
        assert_eq!(
            parse_macos_default_route(scoped),
            Some(MacosDefaultRoute {
                device: "en0".to_string(),
                gateway: Some("fe80::1".parse().unwrap()),
            })
        );
    }

    #[test]
    fn macos_dns_mode_is_writable_only_when_the_snapshot_is_reproducible() {
        let (mode, servers) = parse_macos_dns_configuration(
            "There aren't any DNS Servers set on Wi-Fi.\n",
        );
        assert_eq!(mode, DnsConfigMode::Automatic);
        assert!(servers.is_empty());
        assert!(macos_state_is_configurable(mode, &servers));

        let (mode, servers) = parse_macos_dns_configuration("1.1.1.1\n2606:4700:4700::1111\n");
        assert_eq!(mode, DnsConfigMode::Manual);
        assert_eq!(servers.len(), 2);
        assert!(macos_state_is_configurable(mode, &servers));

        let (mode, servers) =
            parse_macos_dns_configuration("Keine DNS-Server für Wi-Fi konfiguriert.\n");
        assert_eq!(mode, DnsConfigMode::Unknown);
        assert!(servers.is_empty());
        assert!(!macos_state_is_configurable(mode, &servers));
        assert!(!macos_state_is_configurable(
            DnsConfigMode::Manual,
            &[]
        ));
    }

    #[test]
    fn parses_scoped_link_local_gateways_without_losing_the_address() {
        assert_eq!(
            parse_gateway_ip("fe80::abcd%17"),
            Some("fe80::abcd".parse().unwrap())
        );
        assert!(is_ipv6_link_local(&"fe80::abcd".parse().unwrap()));
        assert!(!is_ipv6_link_local(&"2001:db8::1".parse().unwrap()));
    }

    #[test]
    fn scopes_macos_route_state_to_selected_device() {
        let ipv4 = MacosDefaultRoute {
            device: "en0".to_string(),
            gateway: Some("192.168.1.1".parse().unwrap()),
        };
        let ipv6 = MacosDefaultRoute {
            device: "en1".to_string(),
            gateway: Some("2001:db8::1".parse().unwrap()),
        };

        assert_eq!(
            select_macos_route_state("en0", Some(&ipv4), Some(&ipv6)),
            (Some("192.168.1.1".parse().unwrap()), true, false)
        );
        assert_eq!(
            select_macos_route_state("en1", Some(&ipv4), Some(&ipv6)),
            (Some("2001:db8::1".parse().unwrap()), false, true)
        );
        assert_eq!(
            select_macos_route_state("en2", Some(&ipv4), Some(&ipv6)),
            (None, false, false)
        );
    }

    #[test]
    fn windows_route_script_scopes_both_families_and_falls_back_to_ipv6() {
        let script = windows_dns_inspection_script(None);
        assert!(script.contains("if (-not $selectedRoute)"));
        assert!(script.contains("-AddressFamily IPv6 -DestinationPrefix '::/0'"));
        assert!(script.contains("Get-EffectiveRouteMetric"));
        assert!(script.contains("InterfaceMetric"));
        assert_eq!(
            script
                .matches("Where-Object InterfaceIndex -eq $idx")
                .count(),
            2
        );
        assert!(script.contains("IPv4Automatic=$automatic4"));
        assert!(script.contains("IPv6Automatic=$automatic6"));

        let requested = windows_dns_inspection_script(Some("Owner's Wi-Fi"));
        assert!(requested.contains("$requested='Owner''s Wi-Fi'"));
    }

    #[test]
    fn deduplicates_ip_addresses() {
        let values = vec![
            "1.1.1.1".parse().unwrap(),
            "1.0.0.1".parse().unwrap(),
            "1.1.1.1".parse().unwrap(),
        ];
        assert_eq!(
            dedup_ips(values),
            vec![
                "1.1.1.1".parse::<IpAddr>().unwrap(),
                "1.0.0.1".parse::<IpAddr>().unwrap(),
            ]
        );
    }

    #[test]
    fn older_dns_snapshots_default_additive_state() {
        let state: DnsSystemState = serde_json::from_str(
            r#"{
                "interface":"eth0",
                "device":null,
                "interface_index":null,
                "servers":[],
                "configured_servers":[],
                "mode":"unknown",
                "backend":"read-only resolver inspection",
                "gateway":null,
                "ipv6_default_route":false
            }"#,
        )
        .unwrap();
        assert!(!state.ipv4_default_route);
        assert_eq!(state.ipv4_ignore_auto_dns, None);
        assert_eq!(state.ipv6_ignore_auto_dns, None);
        assert_eq!(state.ipv4_automatic, None);
        assert_eq!(state.ipv6_automatic, None);
    }
}
