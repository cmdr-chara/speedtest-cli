use std::{sync::atomic::Ordering, time::Duration};

use anyhow::{Context, Result};
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
    let response = timeout(
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
    let body = response
        .bytes()
        .await
        .context("failed to read DoH response")?;
    super::validate_response(&body, query_id)?;
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
    use crate::model::QualityGrade;

    fn raw_benchmark(successes: usize, samples: Vec<f64>) -> RawBenchmark {
        RawBenchmark {
            provider_id: "doh-test".to_string(),
            provider_name: "DoH test".to_string(),
            profile_name: "test".to_string(),
            category: super::super::DnsCategory::Standard,
            servers: Vec::new(),
            queries: 10,
            successes,
            samples,
            is_current: false,
        }
    }

    #[test]
    fn zero_success_doh_result_has_zero_score_and_f_grade() {
        let result = score_relative(raw_benchmark(0, Vec::new()), 10.0, 10.0);

        assert_eq!(result.score, 0);
        assert_eq!(result.grade, QualityGrade::F);
        assert!(!result.s_tier);
    }

    #[test]
    fn reliable_doh_scoring_is_preserved() {
        let result = score_relative(raw_benchmark(10, vec![10.0; 10]), 10.0, 10.0);

        assert_eq!(result.score, 100);
        assert_eq!(result.grade, QualityGrade::APlus);
        assert!(result.s_tier);
    }

    #[test]
    fn unreliable_doh_latency_does_not_define_relative_baselines() {
        let raw_entries = [
            raw_benchmark(1, vec![1.0]),
            raw_benchmark(10, vec![20.0; 10]),
        ];
        let eligible = raw_entries
            .iter()
            .filter(|raw| percent(raw.successes, raw.queries) >= 80.0)
            .collect::<Vec<_>>();
        let best_median = eligible
            .iter()
            .filter_map(|raw| analysis::distribution(&raw.samples).map(|stats| stats.median_ms))
            .fold(f64::INFINITY, f64::min);
        let best_p95 = eligible
            .iter()
            .filter_map(|raw| analysis::distribution(&raw.samples).map(|stats| stats.p95_ms))
            .fold(f64::INFINITY, f64::min);
        drop(eligible);
        let reliable = score_relative(
            raw_entries.into_iter().nth(1).unwrap(),
            best_median,
            best_p95,
        );
        assert_eq!(reliable.score, 100);
    }
}
