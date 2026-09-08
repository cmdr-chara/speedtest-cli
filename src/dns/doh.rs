use std::{sync::atomic::Ordering, time::Duration};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use reqwest::Client;
use tokio::{
    task::JoinSet,
    time::{timeout, Instant},
};

use crate::analysis;

use super::{
    finalize_score, grade_for_score, median_or_inf, percent, providers_for_profile,
    select_winner_id, BenchmarkProfile, DnsBenchmarkResult, DnsProvider, DnsProviderBenchmark,
    RawBenchmark, DNS_TIMEOUT, NEXT_QUERY_ID, TEST_DOMAINS,
};

pub async fn benchmark(profile: BenchmarkProfile, queries: usize) -> Result<DnsBenchmarkResult> {
    let queries = queries.clamp(3, 100);
    let client = Client::builder()
        .user_agent(concat!("speedtest-cli/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(6))
        .pool_max_idle_per_host(2)
        .build()
        .context("failed to build DoH client")?;
    let mut workers = JoinSet::new();

    for provider in providers_for_profile(profile)
        .into_iter()
        .filter(|provider| provider.doh.is_some())
    {
        let client = client.clone();
        workers.spawn(async move { benchmark_provider(&client, provider, queries).await });
    }

    let mut raw_entries = Vec::new();
    while let Some(result) = workers.join_next().await {
        raw_entries.push(result.context("DoH benchmark worker panicked")?);
    }

    let best_median = raw_entries
        .iter()
        .filter(|raw| percent(raw.successes, raw.queries) >= 80.0)
        .filter_map(|raw| analysis::distribution(&raw.samples).map(|stats| stats.median_ms))
        .fold(f64::INFINITY, f64::min);
    let best_p95 = raw_entries
        .iter()
        .filter(|raw| percent(raw.successes, raw.queries) >= 80.0)
        .filter_map(|raw| analysis::distribution(&raw.samples).map(|stats| stats.p95_ms))
        .fold(f64::INFINITY, f64::min);

    let mut entries = raw_entries
        .into_iter()
        .map(|raw| score_relative(raw, best_median, best_p95))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| median_or_inf(left).total_cmp(&median_or_inf(right)))
    });
    let winner_id = select_winner_id(&entries);

    Ok(DnsBenchmarkResult {
        timestamp: Utc::now(),
        profile: format!("{} / doh", profile.label()),
        queries_per_resolver: queries,
        entries,
        winner_id,
    })
}

async fn benchmark_provider(
    client: &Client,
    provider: &'static DnsProvider,
    queries: usize,
) -> RawBenchmark {
    let endpoint = provider.doh.expect("DoH provider filtered before worker");
    let _ = query_doh(client, endpoint, TEST_DOMAINS[0]).await;

    let mut samples = Vec::with_capacity(queries);
    for index in 0..queries {
        let domain = TEST_DOMAINS[index % TEST_DOMAINS.len()];
        if let Ok(ms) = query_doh(client, endpoint, domain).await {
            samples.push(ms);
        }
    }

    RawBenchmark {
        provider_id: provider.id.to_string(),
        provider_name: provider.provider.to_string(),
        profile_name: provider.profile.to_string(),
        category: provider.category,
        servers: provider.addresses(false),
        queries,
        successes: samples.len(),
        samples,
        is_current: false,
    }
}

async fn query_doh(client: &Client, endpoint: &str, domain: &str) -> Result<f64> {
    let query_id = NEXT_QUERY_ID.fetch_add(1, Ordering::Relaxed);
    let packet = super::build_query(domain, query_id)?;
    let started = Instant::now();
    let mut response = timeout(
        DNS_TIMEOUT.max(Duration::from_secs(3)),
        client
            .post(endpoint)
            .header("accept", "application/dns-message")
            .header("content-type", "application/dns-message")
            .header("cache-control", "no-store")
            .body(packet)
            .send(),
    )
    .await
    .context("DoH query timed out")??
    .error_for_status()
    .context("DoH endpoint returned an error")?;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("failed to read DoH response")?
    {
        if chunk.len() > super::wire::MAX_MESSAGE_LEN - body.len() {
            bail!("DoH response exceeded the DNS message size limit");
        }
        body.extend_from_slice(&chunk);
    }
    super::validate_response(&body, query_id, domain)?;
    Ok(started.elapsed().as_secs_f64() * 1000.0)
}

fn score_relative(raw: RawBenchmark, best_median: f64, best_p95: f64) -> DnsProviderBenchmark {
    let latency = analysis::distribution(&raw.samples);
    let success_rate_percent = percent(raw.successes, raw.queries);
    let median = latency.as_ref().map_or(3_000.0, |stats| stats.median_ms);
    let p95 = latency.as_ref().map_or(3_000.0, |stats| stats.p95_ms);
    let median_score = relative_score(median, best_median);
    let p95_score = relative_score(p95, best_p95);
    let stability_score = latency.as_ref().map_or(0.0, |stats| {
        let spread = (stats.p95_ms - stats.median_ms).max(0.0);
        (100.0 - spread / stats.median_ms.max(1.0) * 100.0).clamp(15.0, 100.0)
    });
    let score = finalize_score(
        raw.successes,
        success_rate_percent,
        median_score * 0.40
            + p95_score * 0.25
            + success_rate_percent * 0.25
            + stability_score * 0.10,
    );
    let grade = grade_for_score(score);
    let s_tier = score >= 98 && success_rate_percent >= 100.0 && median <= best_median * 1.10 + 0.5;

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

fn relative_score(value: f64, best: f64) -> f64 {
    if !best.is_finite() || best <= f64::EPSILON || !value.is_finite() {
        return 0.0;
    }
    (best / value * 100.0).clamp(15.0, 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::wire::MAX_MESSAGE_LEN;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    async fn probe_fixture(reply: fn(&[u8]) -> Vec<u8>) -> Result<f64> {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/dns-query", listener.local_addr().unwrap());
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let fixture = async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(stream.read_u8().await.unwrap());
                assert!(header.len() <= 8192);
            }
            let header = String::from_utf8(header).unwrap().to_ascii_lowercase();
            assert!(header.starts_with("post /dns-query "));
            assert!(header.contains("content-type: application/dns-message"));
            assert!(header.contains("accept: application/dns-message"));
            let length: usize = header
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            assert!(length <= 512);
            let mut query = vec![0; length];
            stream.read_exact(&mut query).await.unwrap();
            let _ = stream.write_all(&reply(&query)).await;
        };
        let (result, ()) = timeout(Duration::from_secs(3), async {
            tokio::join!(query_doh(&client, &endpoint, "example.com"), fixture)
        })
        .await
        .unwrap();
        result
    }

    fn address_response(query: &[u8]) -> Vec<u8> {
        let mut packet = query.to_vec();
        packet[2..4].copy_from_slice(&0x8180_u16.to_be_bytes());
        packet[6..8].copy_from_slice(&1_u16.to_be_bytes());
        packet
            .extend_from_slice(b"\xc0\x0c\x00\x01\x00\x01\x00\x00\x00\x3c\x00\x04\xc0\x00\x02\x01");
        packet
    }

    fn http_response(body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        ).into_bytes();
        response.extend_from_slice(body);
        response
    }

    #[tokio::test]
    async fn doh_requires_a_complete_matching_dns_answer() {
        assert!(
            probe_fixture(|query| http_response(&address_response(query)))
                .await
                .is_ok()
        );
        assert!(probe_fixture(|query| {
            let mut packet = address_response(query);
            packet.truncate(12);
            http_response(&packet)
        })
        .await
        .is_err());
        assert!(probe_fixture(|query| {
            let mut packet = address_response(query);
            packet[13] = b'x';
            http_response(&packet)
        })
        .await
        .is_err());
        assert!(probe_fixture(|query| {
            let mut packet = address_response(query);
            packet[0] ^= 1;
            http_response(&packet)
        })
        .await
        .is_err());
    }

    #[tokio::test]
    async fn doh_rejects_truncated_http_bodies() {
        let result = probe_fixture(|query| {
            let mut response = http_response(&address_response(query));
            response.pop();
            response
        })
        .await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("failed to read DoH response"));
    }

    #[tokio::test]
    async fn doh_bounds_chunked_response_bodies_without_content_length() {
        let result = probe_fixture(|_| {
            let mut response = b"HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
            response.extend_from_slice(format!("{:x}\r\n", MAX_MESSAGE_LEN + 1).as_bytes());
            response.resize(response.len() + MAX_MESSAGE_LEN + 1, 0);
            response.extend_from_slice(b"\r\n0\r\n\r\n");
            response
        }).await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("exceeded the DNS message size limit"));
    }
}
