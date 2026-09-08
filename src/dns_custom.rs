use std::net::IpAddr;

use anyhow::{anyhow, Result};

use crate::{
    analysis,
    dns::{finalize_score, grade_for_score, DnsCategory, DnsProviderBenchmark},
};

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

pub async fn test_servers(servers: Vec<IpAddr>, queries: usize) -> Result<DnsProviderBenchmark> {
    if servers.is_empty() {
        return Err(anyhow!("at least one resolver IP is required"));
    }
    let queries = queries.clamp(3, 100);
    let mut samples = Vec::new();
    for index in 0..queries {
        let server = servers[index % servers.len()];
        let domain = DOMAINS[index % DOMAINS.len()];
        if let Ok(ms) = crate::dns::query_udp(server, domain).await {
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
