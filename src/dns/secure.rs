//! DNS-over-TLS and DNS-over-QUIC active benchmarks.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use rustls::{pki_types::ServerName, ClientConfig, RootCertStore};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{lookup_host, TcpStream},
    task::JoinSet,
    time::{timeout, Instant},
};
use tokio_rustls::TlsConnector;

use crate::{analysis, engine::AddressFamily, model::LatencyDistribution};

use super::{providers_for_profile, BenchmarkProfile, DnsProvider};

const SECURE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecureProtocol {
    Dot,
    Doq,
}

impl SecureProtocol {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Dot => "dot",
            Self::Doq => "doq",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecureDnsBenchmarkResult {
    pub timestamp: DateTime<Utc>,
    pub protocol: SecureProtocol,
    pub profile: String,
    pub queries_per_resolver: usize,
    pub entries: Vec<SecureDnsEntry>,
}

impl SecureDnsBenchmarkResult {
    pub fn pretty_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecureDnsEntry {
    pub provider_id: String,
    pub provider_name: String,
    pub endpoint: String,
    pub queries: usize,
    pub successes: usize,
    pub success_rate_percent: f64,
    pub latency: Option<LatencyDistribution>,
    pub error: Option<String>,
}

pub async fn benchmark(
    profile: BenchmarkProfile,
    queries: usize,
    protocol: SecureProtocol,
    family: AddressFamily,
) -> Result<SecureDnsBenchmarkResult> {
    let queries = queries.clamp(3, 100);
    let mut workers = JoinSet::new();
    for provider in providers_for_profile(profile) {
        let Some(endpoint) = (match protocol {
            SecureProtocol::Dot => provider.dot,
            SecureProtocol::Doq => provider.doq,
        }) else {
            continue;
        };
        workers.spawn(async move {
            benchmark_provider(provider, endpoint, queries, protocol, family).await
        });
    }
    let mut entries = Vec::new();
    while let Some(result) = workers.join_next().await {
        entries.push(result.context("secure DNS benchmark worker panicked")?);
    }
    if entries.is_empty() {
        bail!(
            "no resolver in this profile advertises {}",
            protocol.label()
        );
    }
    entries.sort_by(|left, right| {
        right
            .success_rate_percent
            .total_cmp(&left.success_rate_percent)
            .then_with(|| {
                let l = left
                    .latency
                    .as_ref()
                    .map_or(f64::INFINITY, |latency| latency.median_ms);
                let r = right
                    .latency
                    .as_ref()
                    .map_or(f64::INFINITY, |latency| latency.median_ms);
                l.total_cmp(&r)
            })
            .then_with(|| left.provider_id.cmp(&right.provider_id))
    });
    Ok(SecureDnsBenchmarkResult {
        timestamp: Utc::now(),
        protocol,
        profile: profile.label().to_string(),
        queries_per_resolver: queries,
        entries,
    })
}

async fn benchmark_provider(
    provider: &'static DnsProvider,
    endpoint: &'static str,
    queries: usize,
    protocol: SecureProtocol,
    family: AddressFamily,
) -> SecureDnsEntry {
    let mut samples = Vec::with_capacity(queries);
    let mut error = None;
    for index in 0..queries {
        let domain = super::TEST_DOMAINS[index % super::TEST_DOMAINS.len()];
        match query(provider, endpoint, domain, protocol, family).await {
            Ok(ms) => samples.push(ms),
            Err(err) => error = Some(format!("{err:#}")),
        }
    }
    SecureDnsEntry {
        provider_id: provider.id.to_string(),
        provider_name: provider.display_name(),
        endpoint: endpoint.to_string(),
        queries,
        successes: samples.len(),
        success_rate_percent: samples.len() as f64 / queries as f64 * 100.0,
        latency: analysis::distribution(&samples),
        error,
    }
}

async fn query(
    provider: &'static DnsProvider,
    endpoint: &str,
    domain: &str,
    protocol: SecureProtocol,
    family: AddressFamily,
) -> Result<f64> {
    let query_id = super::NEXT_QUERY_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let packet = super::wire::build_query(domain, query_id)?;
    match protocol {
        SecureProtocol::Dot => query_dot(endpoint, domain, packet, query_id, family).await,
        SecureProtocol::Doq => query_doq(endpoint, domain, packet, query_id, family).await,
    }
    .with_context(|| {
        format!(
            "{} {} query failed",
            provider.display_name(),
            protocol.label()
        )
    })
}

async fn query_dot(
    endpoint: &str,
    domain: &str,
    packet: Vec<u8>,
    query_id: u16,
    family: AddressFamily,
) -> Result<f64> {
    let address = resolve_endpoint(endpoint, 853, family).await?;
    let started = Instant::now();
    let stream = timeout(SECURE_TIMEOUT, TcpStream::connect(address))
        .await
        .context("DoT TCP connection timed out")??;
    let connector = TlsConnector::from(Arc::new(tls_config()));
    let server_name = ServerName::try_from(endpoint.to_string())
        .map_err(|_| anyhow::anyhow!("invalid DoT server name"))?;
    let mut stream = timeout(SECURE_TIMEOUT, connector.connect(server_name, stream))
        .await
        .context("DoT TLS handshake timed out")??;
    let length = u16::try_from(packet.len()).context("DNS query is too large")?;
    timeout(SECURE_TIMEOUT, stream.write_u16(length))
        .await
        .context("DoT query length write timed out")??;
    timeout(SECURE_TIMEOUT, stream.write_all(&packet))
        .await
        .context("DoT query write timed out")??;
    timeout(SECURE_TIMEOUT, stream.flush())
        .await
        .context("DoT query flush timed out")??;
    let response_length = timeout(SECURE_TIMEOUT, stream.read_u16())
        .await
        .context("DoT response length timed out")??;
    let mut response = vec![0; usize::from(response_length)];
    timeout(SECURE_TIMEOUT, stream.read_exact(&mut response))
        .await
        .context("DoT response timed out")??;
    super::wire::validate_response(&response, query_id, domain)?;
    Ok(started.elapsed().as_secs_f64() * 1000.0)
}

async fn query_doq(
    endpoint: &str,
    domain: &str,
    packet: Vec<u8>,
    query_id: u16,
    family: AddressFamily,
) -> Result<f64> {
    let address = resolve_endpoint(endpoint, 853, family).await?;
    let bind = match address {
        SocketAddr::V4(_) => "0.0.0.0:0".parse().expect("valid IPv4 bind"),
        SocketAddr::V6(_) => "[::]:0".parse().expect("valid IPv6 bind"),
    };
    let mut endpoint_client = quinn::Endpoint::client(bind)?;
    let mut crypto = tls_config();
    crypto.alpn_protocols = vec![b"doq".to_vec()];
    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(crypto)?,
    ));
    endpoint_client.set_default_client_config(client_config);
    let started = Instant::now();
    let connecting = endpoint_client.connect(address, endpoint)?;
    let connection = timeout(SECURE_TIMEOUT, connecting)
        .await
        .context("DoQ connection timed out")??;
    let (mut send, mut receive) = timeout(SECURE_TIMEOUT, connection.open_bi())
        .await
        .context("DoQ stream timed out")??;
    let length = u16::try_from(packet.len()).context("DNS query is too large")?;
    timeout(SECURE_TIMEOUT, send.write_u16(length))
        .await
        .context("DoQ query length write timed out")??;
    timeout(SECURE_TIMEOUT, send.write_all(&packet))
        .await
        .context("DoQ query write timed out")??;
    send.finish()?;
    let response = timeout(SECURE_TIMEOUT, receive.read_to_end(65_535))
        .await
        .context("DoQ response timed out")??;
    if response.len() < 2 {
        bail!("DoQ response omitted its length prefix");
    }
    let response_length = usize::from(u16::from_be_bytes([response[0], response[1]]));
    if response_length != response.len() - 2 {
        bail!("DoQ response length prefix did not match its payload");
    }
    super::wire::validate_response(&response[2..], query_id, domain)?;
    connection.close(0u32.into(), b"done");
    timeout(SECURE_TIMEOUT, endpoint_client.wait_idle())
        .await
        .context("DoQ endpoint shutdown timed out")?;
    Ok(started.elapsed().as_secs_f64() * 1000.0)
}

async fn resolve_endpoint(endpoint: &str, port: u16, family: AddressFamily) -> Result<SocketAddr> {
    timeout(SECURE_TIMEOUT, lookup_host((endpoint, port)))
        .await
        .context("secure DNS endpoint lookup timed out")??
        .find(|address| match family {
            AddressFamily::Any => true,
            AddressFamily::Ipv4 => address.is_ipv4(),
            AddressFamily::Ipv6 => address.is_ipv6(),
        })
        .context("secure DNS endpoint has no address for the requested family")
}

fn tls_config() -> ClientConfig {
    let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_labels_are_stable() {
        assert_eq!(SecureProtocol::Dot.label(), "dot");
        assert_eq!(SecureProtocol::Doq.label(), "doq");
    }

    #[tokio::test]
    async fn endpoint_resolution_honors_ipv4_selection() {
        let address = resolve_endpoint("127.0.0.1", 853, AddressFamily::Ipv4)
            .await
            .unwrap();
        assert!(address.ip().is_ipv4());
    }
}
