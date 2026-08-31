use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::atomic::{AtomicU16, Ordering},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use tokio::{
    net::UdpSocket,
    time::{timeout, Instant},
};

use crate::{
    analysis,
    dns::{finalize_score, grade_for_score, DnsCategory, DnsProviderBenchmark},
};

const TIMEOUT: Duration = Duration::from_millis(1_500);
const DOMAINS: [&str; 8] = [
    "cloudflare.com",
    "google.com",
    "wikipedia.org",
    "github.com",
    "microsoft.com",
    "apple.com",
    "rust-lang.org",
    "ietf.org",
];
static NEXT_ID: AtomicU16 = AtomicU16::new(0x7221);

pub async fn test_servers(servers: Vec<IpAddr>, queries: usize) -> Result<DnsProviderBenchmark> {
    if servers.is_empty() {
        return Err(anyhow!("at least one resolver IP is required"));
    }
    let queries = queries.clamp(3, 100);
    let mut samples = Vec::new();
    for index in 0..queries {
        let server = servers[index % servers.len()];
        let domain = DOMAINS[index % DOMAINS.len()];
        if let Ok(ms) = query(server, domain).await {
            samples.push(ms);
        }
    }
    Ok(score_samples(servers, queries, &samples))
}

fn score_samples(servers: Vec<IpAddr>, queries: usize, samples: &[f64]) -> DnsProviderBenchmark {
    let successes = samples.len();
    let success_rate_percent = successes as f64 / queries as f64 * 100.0;
    let latency = analysis::distribution(samples);
    let median = latency.as_ref().map_or(1_500.0, |stats| stats.median_ms);
    let p95 = latency.as_ref().map_or(1_500.0, |stats| stats.p95_ms);
    let spread = latency
        .as_ref()
        .map_or(1_500.0, |stats| (stats.p95_ms - stats.median_ms).max(0.0));
    let latency_score = absolute_latency_score(median) * 0.60 + absolute_latency_score(p95) * 0.25;
    let stability_score = (100.0 - spread / median.max(1.0) * 100.0).clamp(15.0, 100.0);
    let score = finalize_score(
        successes,
        success_rate_percent,
        latency_score + success_rate_percent * 0.10 + stability_score * 0.05,
    );
    let grade = grade_for_score(score);

    DnsProviderBenchmark {
        provider_id: "custom".to_string(),
        provider_name: "Custom resolver".to_string(),
        profile_name: "explicit IP".to_string(),
        category: DnsCategory::Standard,
        servers,
        queries,
        successes,
        success_rate_percent,
        latency,
        score,
        grade,
        s_tier: score >= 98 && success_rate_percent >= 100.0 && median <= 15.0,
        is_current: false,
    }
}

async fn query(server: IpAddr, domain: &str) -> Result<f64> {
    let bind = match server {
        IpAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        IpAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    };
    let socket = UdpSocket::bind(bind)
        .await
        .context("failed to bind DNS socket")?;
    socket
        .connect(SocketAddr::new(server, 53))
        .await
        .context("failed to connect DNS socket")?;
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let packet = build_query(domain, id)?;
    let started = Instant::now();
    timeout(TIMEOUT, socket.send(&packet))
        .await
        .context("DNS send timed out")??;
    let mut response = [0_u8; 4096];
    let size = timeout(TIMEOUT, socket.recv(&mut response))
        .await
        .context("DNS response timed out")??;
    validate(&response[..size], id)?;
    Ok(started.elapsed().as_secs_f64() * 1000.0)
}

fn build_query(domain: &str, id: u16) -> Result<Vec<u8>> {
    let mut packet = Vec::with_capacity(128);
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&0x0100_u16.to_be_bytes());
    packet.extend_from_slice(&1_u16.to_be_bytes());
    packet.extend_from_slice(&[0_u8; 6]);
    for label in domain.split('.') {
        if label.is_empty() || label.len() > 63 || !label.is_ascii() {
            return Err(anyhow!("invalid DNS name: {domain}"));
        }
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&1_u16.to_be_bytes());
    packet.extend_from_slice(&1_u16.to_be_bytes());
    Ok(packet)
}

fn validate(response: &[u8], id: u16) -> Result<()> {
    if response.len() < 12 || u16::from_be_bytes([response[0], response[1]]) != id {
        return Err(anyhow!("invalid DNS response header"));
    }
    let flags = u16::from_be_bytes([response[2], response[3]]);
    let answers = u16::from_be_bytes([response[6], response[7]]);
    if flags & 0x8000 == 0 || flags & 0x000f != 0 || answers == 0 {
        return Err(anyhow!("DNS resolver returned an unusable response"));
    }
    Ok(())
}

fn absolute_latency_score(value: f64) -> f64 {
    if value <= 10.0 {
        100.0
    } else if value <= 20.0 {
        95.0
    } else if value <= 35.0 {
        88.0
    } else if value <= 60.0 {
        75.0
    } else if value <= 100.0 {
        55.0
    } else if value <= 200.0 {
        35.0
    } else {
        15.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::QualityGrade;

    #[test]
    fn zero_success_custom_result_has_zero_score_and_f_grade() {
        let result = score_samples(vec!["192.0.2.1".parse().unwrap()], 10, &[]);

        assert_eq!(result.score, 0);
        assert_eq!(result.grade, QualityGrade::F);
        assert!(!result.s_tier);
    }

    #[test]
    fn reliable_custom_scoring_is_preserved() {
        let result = score_samples(vec!["192.0.2.1".parse().unwrap()], 10, &[10.0; 10]);

        assert_eq!(result.score, 100);
        assert_eq!(result.grade, QualityGrade::APlus);
        assert!(result.s_tier);
    }

    #[test]
    fn partial_custom_success_cannot_receive_an_elite_grade() {
        let result = score_samples(vec!["192.0.2.1".parse().unwrap()], 10, &[5.0; 5]);
        assert!(result.score <= 40);
        assert_eq!(result.grade, QualityGrade::F);
    }
}
