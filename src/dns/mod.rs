pub mod system;

use std::{
    fs,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::atomic::{AtomicU16, Ordering},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::{
    net::UdpSocket,
    task::JoinSet,
    time::{timeout, Instant},
};

use crate::{
    analysis,
    model::{LatencyDistribution, QualityGrade},
    storage,
};

const DNS_TIMEOUT: Duration = Duration::from_millis(1_500);
const DNS_PORT: u16 = 53;
const TEST_DOMAINS: [&str; 10] = [
    "cloudflare.com",
    "google.com",
    "wikipedia.org",
    "github.com",
    "microsoft.com",
    "apple.com",
    "amazon.com",
    "mozilla.org",
    "rust-lang.org",
    "ietf.org",
];

static NEXT_QUERY_ID: AtomicU16 = AtomicU16::new(0x4a31);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsCategory {
    Standard,
    Security,
    Adblock,
    Family,
}

impl DnsCategory {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Security => "security",
            Self::Adblock => "ad-blocking",
            Self::Family => "family",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkProfile {
    Fastest,
    Privacy,
    Security,
    Adblock,
    Family,
    All,
}

impl BenchmarkProfile {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fastest => "fastest",
            Self::Privacy => "privacy",
            Self::Security => "security",
            Self::Adblock => "adblock",
            Self::Family => "family",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DnsProvider {
    pub id: &'static str,
    pub provider: &'static str,
    pub profile: &'static str,
    pub category: DnsCategory,
    pub privacy_oriented: bool,
    pub ipv4: &'static [&'static str],
    pub ipv6: &'static [&'static str],
    pub doh: Option<&'static str>,
    pub dot: Option<&'static str>,
    pub doq: Option<&'static str>,
    pub dnssec: bool,
}

impl DnsProvider {
    pub fn display_name(self) -> String {
        if self.profile.eq_ignore_ascii_case("standard") {
            self.provider.to_string()
        } else {
            format!("{} {}", self.provider, self.profile)
        }
    }

    pub fn addresses(self, include_ipv6: bool) -> Vec<IpAddr> {
        let mut addresses = self
            .ipv4
            .iter()
            .filter_map(|address| address.parse::<IpAddr>().ok())
            .collect::<Vec<_>>();
        if include_ipv6 {
            addresses.extend(
                self.ipv6
                    .iter()
                    .filter_map(|address| address.parse::<IpAddr>().ok()),
            );
        }
        addresses
    }
}

pub const PROVIDERS: &[DnsProvider] = &[
    DnsProvider {
        id: "cloudflare",
        provider: "Cloudflare",
        profile: "standard",
        category: DnsCategory::Standard,
        privacy_oriented: true,
        ipv4: &["1.1.1.1", "1.0.0.1"],
        ipv6: &["2606:4700:4700::1111", "2606:4700:4700::1001"],
        doh: Some("https://cloudflare-dns.com/dns-query"),
        dot: Some("one.one.one.one"),
        doq: None,
        dnssec: true,
    },
    DnsProvider {
        id: "cloudflare-malware",
        provider: "Cloudflare",
        profile: "malware",
        category: DnsCategory::Security,
        privacy_oriented: true,
        ipv4: &["1.1.1.2", "1.0.0.2"],
        ipv6: &["2606:4700:4700::1112", "2606:4700:4700::1002"],
        doh: Some("https://security.cloudflare-dns.com/dns-query"),
        dot: Some("security.cloudflare-dns.com"),
        doq: None,
        dnssec: true,
    },
    DnsProvider {
        id: "cloudflare-family",
        provider: "Cloudflare",
        profile: "family",
        category: DnsCategory::Family,
        privacy_oriented: true,
        ipv4: &["1.1.1.3", "1.0.0.3"],
        ipv6: &["2606:4700:4700::1113", "2606:4700:4700::1003"],
        doh: Some("https://family.cloudflare-dns.com/dns-query"),
        dot: Some("family.cloudflare-dns.com"),
        doq: None,
        dnssec: true,
    },
    DnsProvider {
        id: "google",
        provider: "Google Public DNS",
        profile: "standard",
        category: DnsCategory::Standard,
        privacy_oriented: false,
        ipv4: &["8.8.8.8", "8.8.4.4"],
        ipv6: &["2001:4860:4860::8888", "2001:4860:4860::8844"],
        doh: Some("https://dns.google/dns-query"),
        dot: Some("dns.google"),
        doq: None,
        dnssec: true,
    },
    DnsProvider {
        id: "quad9",
        provider: "Quad9",
        profile: "secure",
        category: DnsCategory::Security,
        privacy_oriented: true,
        ipv4: &["9.9.9.9", "149.112.112.112"],
        ipv6: &["2620:fe::fe", "2620:fe::9"],
        doh: Some("https://dns.quad9.net/dns-query"),
        dot: Some("dns.quad9.net"),
        doq: None,
        dnssec: true,
    },
    DnsProvider {
        id: "quad9-ecs",
        provider: "Quad9",
        profile: "secure + ECS",
        category: DnsCategory::Security,
        privacy_oriented: false,
        ipv4: &["9.9.9.11", "149.112.112.11"],
        ipv6: &["2620:fe::11", "2620:fe::fe:11"],
        doh: Some("https://dns11.quad9.net/dns-query"),
        dot: Some("dns11.quad9.net"),
        doq: None,
        dnssec: true,
    },
    DnsProvider {
        id: "quad9-unsecured",
        provider: "Quad9",
        profile: "unfiltered",
        category: DnsCategory::Standard,
        privacy_oriented: true,
        ipv4: &["9.9.9.10", "149.112.112.10"],
        ipv6: &["2620:fe::10", "2620:fe::fe:10"],
        doh: Some("https://dns10.quad9.net/dns-query"),
        dot: Some("dns10.quad9.net"),
        doq: None,
        dnssec: true,
    },
    DnsProvider {
        id: "controld",
        provider: "Control D",
        profile: "standard",
        category: DnsCategory::Standard,
        privacy_oriented: true,
        ipv4: &["76.76.2.0", "76.76.10.0"],
        ipv6: &["2606:1a40::", "2606:1a40:1::"],
        doh: Some("https://freedns.controld.com/p0"),
        dot: Some("p0.freedns.controld.com"),
        doq: Some("p0.freedns.controld.com"),
        dnssec: true,
    },
    DnsProvider {
        id: "controld-malware",
        provider: "Control D",
        profile: "malware",
        category: DnsCategory::Security,
        privacy_oriented: true,
        ipv4: &["76.76.2.1", "76.76.10.1"],
        ipv6: &["2606:1a40::1", "2606:1a40:1::1"],
        doh: Some("https://freedns.controld.com/p1"),
        dot: Some("p1.freedns.controld.com"),
        doq: Some("p1.freedns.controld.com"),
        dnssec: true,
    },
    DnsProvider {
        id: "controld-ads",
        provider: "Control D",
        profile: "ads & tracking",
        category: DnsCategory::Adblock,
        privacy_oriented: true,
        ipv4: &["76.76.2.2", "76.76.10.2"],
        ipv6: &["2606:1a40::2", "2606:1a40:1::2"],
        doh: Some("https://freedns.controld.com/p2"),
        dot: Some("p2.freedns.controld.com"),
        doq: Some("p2.freedns.controld.com"),
        dnssec: true,
    },
    DnsProvider {
        id: "controld-family",
        provider: "Control D",
        profile: "family",
        category: DnsCategory::Family,
        privacy_oriented: true,
        ipv4: &["76.76.2.4", "76.76.10.4"],
        ipv6: &["2606:1a40::4", "2606:1a40:1::4"],
        doh: Some("https://freedns.controld.com/family"),
        dot: Some("family.freedns.controld.com"),
        doq: Some("family.freedns.controld.com"),
        dnssec: true,
    },
    DnsProvider {
        id: "adguard-unfiltered",
        provider: "AdGuard DNS",
        profile: "unfiltered",
        category: DnsCategory::Standard,
        privacy_oriented: true,
        ipv4: &["94.140.14.140", "94.140.14.141"],
        ipv6: &["2a10:50c0::1:ff", "2a10:50c0::2:ff"],
        doh: Some("https://unfiltered.adguard-dns.com/dns-query"),
        dot: Some("unfiltered.adguard-dns.com"),
        doq: Some("unfiltered.adguard-dns.com"),
        dnssec: true,
    },
    DnsProvider {
        id: "adguard",
        provider: "AdGuard DNS",
        profile: "ads & tracking",
        category: DnsCategory::Adblock,
        privacy_oriented: true,
        ipv4: &["94.140.14.14", "94.140.15.15"],
        ipv6: &["2a10:50c0::ad1:ff", "2a10:50c0::ad2:ff"],
        doh: Some("https://dns.adguard-dns.com/dns-query"),
        dot: Some("dns.adguard-dns.com"),
        doq: Some("dns.adguard-dns.com"),
        dnssec: true,
    },
    DnsProvider {
        id: "adguard-family",
        provider: "AdGuard DNS",
        profile: "family",
        category: DnsCategory::Family,
        privacy_oriented: true,
        ipv4: &["94.140.14.15", "94.140.15.16"],
        ipv6: &["2a10:50c0::bad1:ff", "2a10:50c0::bad2:ff"],
        doh: Some("https://family.adguard-dns.com/dns-query"),
        dot: Some("family.adguard-dns.com"),
        doq: Some("family.adguard-dns.com"),
        dnssec: true,
    },
    DnsProvider {
        id: "cleanbrowsing-security",
        provider: "CleanBrowsing",
        profile: "security",
        category: DnsCategory::Security,
        privacy_oriented: true,
        ipv4: &["185.228.168.9", "185.228.169.9"],
        ipv6: &["2a0d:2a00:1::2", "2a0d:2a00:2::2"],
        doh: Some("https://doh.cleanbrowsing.org/doh/security-filter/"),
        dot: Some("security-filter-dns.cleanbrowsing.org"),
        doq: None,
        dnssec: true,
    },
    DnsProvider {
        id: "cleanbrowsing-adult",
        provider: "CleanBrowsing",
        profile: "adult",
        category: DnsCategory::Family,
        privacy_oriented: true,
        ipv4: &["185.228.168.10", "185.228.169.11"],
        ipv6: &["2a0d:2a00:1::1", "2a0d:2a00:2::1"],
        doh: Some("https://doh.cleanbrowsing.org/doh/adult-filter/"),
        dot: Some("adult-filter-dns.cleanbrowsing.org"),
        doq: None,
        dnssec: true,
    },
    DnsProvider {
        id: "cleanbrowsing-family",
        provider: "CleanBrowsing",
        profile: "family",
        category: DnsCategory::Family,
        privacy_oriented: true,
        ipv4: &["185.228.168.168", "185.228.169.168"],
        ipv6: &["2a0d:2a00:1::", "2a0d:2a00:2::"],
        doh: Some("https://doh.cleanbrowsing.org/doh/family-filter/"),
        dot: Some("family-filter-dns.cleanbrowsing.org"),
        doq: None,
        dnssec: true,
    },
    DnsProvider {
        id: "opendns",
        provider: "OpenDNS",
        profile: "standard",
        category: DnsCategory::Standard,
        privacy_oriented: false,
        ipv4: &["208.67.222.222", "208.67.220.220"],
        ipv6: &[],
        doh: None,
        dot: None,
        doq: None,
        dnssec: false,
    },
    DnsProvider {
        id: "opendns-family",
        provider: "OpenDNS",
        profile: "FamilyShield",
        category: DnsCategory::Family,
        privacy_oriented: false,
        ipv4: &["208.67.222.123", "208.67.220.123"],
        ipv6: &[],
        doh: None,
        dot: None,
        doq: None,
        dnssec: false,
    },
    DnsProvider {
        id: "dns-sb",
        provider: "DNS.SB",
        profile: "standard",
        category: DnsCategory::Standard,
        privacy_oriented: true,
        ipv4: &["185.222.222.222", "45.11.45.11"],
        ipv6: &["2a09::", "2a11::"],
        doh: Some("https://doh.dns.sb/dns-query"),
        dot: Some("dot.sb"),
        doq: None,
        dnssec: true,
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsProviderBenchmark {
    pub provider_id: String,
    pub provider_name: String,
    pub profile_name: String,
    pub category: DnsCategory,
    pub servers: Vec<IpAddr>,
    pub queries: usize,
    pub successes: usize,
    pub success_rate_percent: f64,
    pub latency: Option<LatencyDistribution>,
    pub score: u8,
    pub grade: QualityGrade,
    pub s_tier: bool,
    pub is_current: bool,
}

impl DnsProviderBenchmark {
    pub fn tier_label(&self) -> Option<&'static str> {
        self.s_tier.then_some("S-TIER")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsBenchmarkResult {
    pub timestamp: DateTime<Utc>,
    pub profile: String,
    pub queries_per_resolver: usize,
    pub entries: Vec<DnsProviderBenchmark>,
    pub winner_id: Option<String>,
}

impl DnsBenchmarkResult {
    pub fn winner(&self) -> Option<&DnsProviderBenchmark> {
        let winner = self.winner_id.as_deref()?;
        self.entries
            .iter()
            .find(|entry| entry.provider_id == winner)
    }

    pub fn pretty_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsBackup {
    pub timestamp: DateTime<Utc>,
    pub state: system::DnsSystemState,
}

pub fn provider(id: &str) -> Option<&'static DnsProvider> {
    let normalized = match id.to_ascii_lowercase().as_str() {
        "cloudflare-standard" => "cloudflare",
        "google-public-dns" => "google",
        "quad9-secure" => "quad9",
        "controld-unfiltered" => "controld",
        "adguard-default" => "adguard",
        "cleanbrowsing" => "cleanbrowsing-security",
        "familyshield" => "opendns-family",
        other => other,
    };
    PROVIDERS.iter().find(|provider| provider.id == normalized)
}

pub fn providers_for_profile(profile: BenchmarkProfile) -> Vec<&'static DnsProvider> {
    PROVIDERS
        .iter()
        .filter(|provider| match profile {
            BenchmarkProfile::Fastest => provider.category == DnsCategory::Standard,
            BenchmarkProfile::Privacy => {
                provider.category == DnsCategory::Standard && provider.privacy_oriented
            }
            BenchmarkProfile::Security => provider.category == DnsCategory::Security,
            BenchmarkProfile::Adblock => provider.category == DnsCategory::Adblock,
            BenchmarkProfile::Family => provider.category == DnsCategory::Family,
            BenchmarkProfile::All => true,
        })
        .collect()
}

pub async fn test_current(queries: usize) -> Result<DnsProviderBenchmark> {
    let state = system::inspect(None)?;
    if state.servers.is_empty() {
        return Err(anyhow!(
            "the active interface did not expose any DNS servers"
        ));
    }
    let raw = benchmark_servers(
        "current",
        "Current / System DNS",
        "active",
        DnsCategory::Standard,
        state.servers,
        queries,
        true,
    )
    .await;
    Ok(score_single(raw))
}

pub async fn benchmark(profile: BenchmarkProfile, queries: usize) -> Result<DnsBenchmarkResult> {
    let queries = queries.clamp(3, 100);
    let mut workers = JoinSet::new();

    for provider in providers_for_profile(profile) {
        workers.spawn(async move {
            let addresses = provider.addresses(false);
            benchmark_servers(
                provider.id,
                provider.provider,
                provider.profile,
                provider.category,
                addresses,
                queries,
                false,
            )
            .await
        });
    }

    let mut raw_entries = Vec::new();
    while let Some(result) = workers.join_next().await {
        raw_entries.push(result.context("DNS benchmark worker panicked")?);
    }

    if matches!(profile, BenchmarkProfile::Fastest | BenchmarkProfile::All) {
        if let Ok(state) = system::inspect(None) {
            if !state.servers.is_empty() {
                raw_entries.push(
                    benchmark_servers(
                        "current",
                        "Current / System DNS",
                        "active",
                        DnsCategory::Standard,
                        state.servers,
                        queries,
                        true,
                    )
                    .await,
                );
            }
        }
    }

    let mut entries = score_entries(raw_entries);
    entries.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| median_or_inf(left).total_cmp(&median_or_inf(right)))
    });
    let winner_id = entries
        .iter()
        .find(|entry| entry.successes > 0)
        .map(|entry| entry.provider_id.clone());

    Ok(DnsBenchmarkResult {
        timestamp: Utc::now(),
        profile: profile.label().to_string(),
        queries_per_resolver: queries,
        entries,
        winner_id,
    })
}

async fn benchmark_servers(
    id: &str,
    provider_name: &str,
    profile_name: &str,
    category: DnsCategory,
    servers: Vec<IpAddr>,
    queries: usize,
    is_current: bool,
) -> RawBenchmark {
    let mut samples = Vec::new();
    if servers.is_empty() {
        return RawBenchmark {
            provider_id: id.to_string(),
            provider_name: provider_name.to_string(),
            profile_name: profile_name.to_string(),
            category,
            servers,
            queries,
            successes: 0,
            samples,
            is_current,
        };
    }

    for index in 0..queries {
        let server = servers[index % servers.len()];
        let domain = TEST_DOMAINS[index % TEST_DOMAINS.len()];
        if let Ok(latency_ms) = query_udp(server, domain).await {
            samples.push(latency_ms);
        }
    }

    RawBenchmark {
        provider_id: id.to_string(),
        provider_name: provider_name.to_string(),
        profile_name: profile_name.to_string(),
        category,
        servers,
        queries,
        successes: samples.len(),
        samples,
        is_current,
    }
}

#[derive(Debug)]
struct RawBenchmark {
    provider_id: String,
    provider_name: String,
    profile_name: String,
    category: DnsCategory,
    servers: Vec<IpAddr>,
    queries: usize,
    successes: usize,
    samples: Vec<f64>,
    is_current: bool,
}

fn score_single(raw: RawBenchmark) -> DnsProviderBenchmark {
    let latency = analysis::distribution(&raw.samples);
    let success_rate_percent = percent(raw.successes, raw.queries);
    let median = latency.as_ref().map_or(1_500.0, |stats| stats.median_ms);
    let p95 = latency.as_ref().map_or(1_500.0, |stats| stats.p95_ms);
    let median_score = absolute_latency_score(median);
    let p95_score = absolute_latency_score(p95);
    let stability_score = latency.as_ref().map_or(0.0, spread_score);
    let score = (median_score * 0.40
        + p95_score * 0.25
        + success_rate_percent * 0.20
        + stability_score * 0.10
        + f64::from(raw.successes > 0) * 100.0 * 0.05)
        .round()
        .clamp(0.0, 100.0) as u8;
    let grade = grade_for_score(score);
    let s_tier = score >= 98 && success_rate_percent >= 100.0 && median <= 15.0;

    DnsProviderBenchmark {
        provider_id: raw.provider_id,
        provider_name: raw.provider_name,
        profile_name: raw.profile_name,
        category: raw.category,
        servers: raw.servers,
        queries: raw.queries,
        successes: raw.successes,
        success_rate_percent,
        latency,
        score,
        grade,
        s_tier,
        is_current: raw.is_current,
    }
}

fn score_entries(raw_entries: Vec<RawBenchmark>) -> Vec<DnsProviderBenchmark> {
    let best_median = raw_entries
        .iter()
        .filter_map(|raw| analysis::distribution(&raw.samples).map(|stats| stats.median_ms))
        .fold(f64::INFINITY, f64::min);
    let best_p95 = raw_entries
        .iter()
        .filter_map(|raw| analysis::distribution(&raw.samples).map(|stats| stats.p95_ms))
        .fold(f64::INFINITY, f64::min);

    raw_entries
        .into_iter()
        .map(|raw| {
            let latency = analysis::distribution(&raw.samples);
            let success_rate_percent = percent(raw.successes, raw.queries);
            let median = latency.as_ref().map_or(1_500.0, |stats| stats.median_ms);
            let p95 = latency.as_ref().map_or(1_500.0, |stats| stats.p95_ms);
            let median_score = relative_latency_score(median, best_median);
            let p95_score = relative_latency_score(p95, best_p95);
            let stability_score = latency.as_ref().map_or(0.0, spread_score);
            let correctness_score = if raw.successes > 0 { 100.0 } else { 0.0 };
            let score = (median_score * 0.40
                + p95_score * 0.25
                + success_rate_percent * 0.20
                + stability_score * 0.10
                + correctness_score * 0.05)
                .round()
                .clamp(0.0, 100.0) as u8;
            let grade = grade_for_score(score);
            let s_tier =
                score >= 98 && success_rate_percent >= 100.0 && median <= best_median * 1.10 + 0.25;

            DnsProviderBenchmark {
                provider_id: raw.provider_id,
                provider_name: raw.provider_name,
                profile_name: raw.profile_name,
                category: raw.category,
                servers: raw.servers,
                queries: raw.queries,
                successes: raw.successes,
                success_rate_percent,
                latency,
                score,
                grade,
                s_tier,
                is_current: raw.is_current,
            }
        })
        .collect()
}

fn relative_latency_score(value: f64, best: f64) -> f64 {
    if !best.is_finite() || best <= f64::EPSILON || !value.is_finite() {
        return 0.0;
    }
    (best / value * 100.0).clamp(15.0, 100.0)
}

fn absolute_latency_score(value: f64) -> f64 {
    match value {
        value if value <= 10.0 => 100.0,
        value if value <= 20.0 => 95.0,
        value if value <= 35.0 => 88.0,
        value if value <= 60.0 => 75.0,
        value if value <= 100.0 => 55.0,
        value if value <= 200.0 => 35.0,
        _ => 15.0,
    }
}

fn spread_score(stats: &LatencyDistribution) -> f64 {
    let spread = (stats.p95_ms - stats.median_ms).max(0.0);
    let relative = spread / stats.median_ms.max(1.0);
    (100.0 - relative * 100.0).clamp(15.0, 100.0)
}

fn grade_for_score(score: u8) -> QualityGrade {
    if score >= 95 {
        QualityGrade::APlus
    } else if score >= 88 {
        QualityGrade::A
    } else if score >= 78 {
        QualityGrade::B
    } else if score >= 65 {
        QualityGrade::C
    } else if score >= 50 {
        QualityGrade::D
    } else {
        QualityGrade::F
    }
}

fn median_or_inf(entry: &DnsProviderBenchmark) -> f64 {
    entry
        .latency
        .as_ref()
        .map_or(f64::INFINITY, |stats| stats.median_ms)
}

fn percent(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64 * 100.0
    }
}

async fn query_udp(server: IpAddr, domain: &str) -> Result<f64> {
    let bind_address = match server {
        IpAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        IpAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    };
    let socket = UdpSocket::bind(bind_address)
        .await
        .context("failed to create DNS UDP socket")?;
    socket
        .connect(SocketAddr::new(server, DNS_PORT))
        .await
        .with_context(|| format!("failed to connect DNS socket to {server}"))?;

    let query_id = NEXT_QUERY_ID.fetch_add(1, Ordering::Relaxed);
    let packet = build_query(domain, query_id)?;
    let started = Instant::now();
    timeout(DNS_TIMEOUT, socket.send(&packet))
        .await
        .context("DNS send timed out")??;

    let mut response = [0_u8; 4096];
    let size = timeout(DNS_TIMEOUT, socket.recv(&mut response))
        .await
        .context("DNS response timed out")??;
    validate_response(&response[..size], query_id)?;
    Ok(started.elapsed().as_secs_f64() * 1000.0)
}

fn build_query(domain: &str, query_id: u16) -> Result<Vec<u8>> {
    let mut packet = Vec::with_capacity(512);
    packet.extend_from_slice(&query_id.to_be_bytes());
    packet.extend_from_slice(&0x0100_u16.to_be_bytes());
    packet.extend_from_slice(&1_u16.to_be_bytes());
    packet.extend_from_slice(&0_u16.to_be_bytes());
    packet.extend_from_slice(&0_u16.to_be_bytes());
    packet.extend_from_slice(&0_u16.to_be_bytes());

    for label in domain.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 || !label.is_ascii() {
            return Err(anyhow!("invalid DNS test name: {domain}"));
        }
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&1_u16.to_be_bytes());
    packet.extend_from_slice(&1_u16.to_be_bytes());
    Ok(packet)
}

fn validate_response(response: &[u8], query_id: u16) -> Result<()> {
    if response.len() < 12 {
        return Err(anyhow!("DNS response was shorter than the header"));
    }
    if u16::from_be_bytes([response[0], response[1]]) != query_id {
        return Err(anyhow!("DNS response transaction ID did not match"));
    }
    let flags = u16::from_be_bytes([response[2], response[3]]);
    if flags & 0x8000 == 0 {
        return Err(anyhow!("DNS packet was not a response"));
    }
    if flags & 0x000f != 0 {
        return Err(anyhow!("DNS resolver returned rcode {}", flags & 0x000f));
    }
    let answers = u16::from_be_bytes([response[6], response[7]]);
    if answers == 0 {
        return Err(anyhow!("DNS resolver returned no answers"));
    }
    Ok(())
}

pub fn save_backup(state: &system::DnsSystemState) -> Result<std::path::PathBuf> {
    let root = storage::data_root()?;
    let path = root.join("dns").join("last-backup.json");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("failed to create DNS backup directory")?;
    }
    let backup = DnsBackup {
        timestamp: Utc::now(),
        state: state.clone(),
    };
    fs::write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(&backup)?),
    )
    .context("failed to write DNS rollback snapshot")?;
    Ok(path)
}

pub fn load_backup() -> Result<DnsBackup> {
    let path = storage::data_root()?.join("dns").join("last-backup.json");
    let content = fs::read_to_string(&path)
        .with_context(|| format!("no DNS rollback snapshot found at {}", path.display()))?;
    serde_json::from_str(&content).context("failed to parse DNS rollback snapshot")
}

pub async fn verify_system_resolution() -> Result<()> {
    let mut resolved = timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host(("example.com", 443)),
    )
    .await
    .context("system DNS verification timed out")??;
    resolved
        .next()
        .ok_or_else(|| anyhow!("system resolver returned no addresses"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_registry_has_unique_ids_and_multiple_leagues() {
        let mut ids = PROVIDERS
            .iter()
            .map(|provider| provider.id)
            .collect::<Vec<_>>();
        let original_len = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), original_len);
        assert!(original_len >= 20);
        assert!(providers_for_profile(BenchmarkProfile::Fastest).len() >= 6);
        assert!(providers_for_profile(BenchmarkProfile::Security).len() >= 4);
        assert!(providers_for_profile(BenchmarkProfile::Family).len() >= 5);
    }

    #[test]
    fn builds_and_validates_basic_dns_packet_shape() {
        let packet = build_query("example.com", 0x1234).unwrap();
        assert_eq!(&packet[..2], &[0x12, 0x34]);
        assert!(packet.windows(7).any(|part| part == b"example"));

        let mut response = [0_u8; 12];
        response[..2].copy_from_slice(&0x1234_u16.to_be_bytes());
        response[2..4].copy_from_slice(&0x8180_u16.to_be_bytes());
        response[6..8].copy_from_slice(&1_u16.to_be_bytes());
        assert!(validate_response(&response, 0x1234).is_ok());
    }

    #[test]
    fn common_provider_aliases_resolve() {
        assert_eq!(provider("cloudflare").unwrap().id, "cloudflare");
        assert_eq!(provider("quad9-secure").unwrap().id, "quad9");
        assert_eq!(provider("familyshield").unwrap().id, "opendns-family");
    }
}
