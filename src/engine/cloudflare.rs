use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::{header::RETRY_AFTER, Client, Response, StatusCode};
use tokio::{
    sync::mpsc::UnboundedSender,
    task::JoinSet,
    time::{sleep, sleep_until, Instant},
};

use crate::{
    engine::{EngineConfig, EngineEvent},
    model::{LatencyResult, ServerInfo, TestPhase, TestResult, ThroughputResult},
};

const DOWNLOAD_URL: &str = "https://speed.cloudflare.com/__down";
const UPLOAD_URL: &str = "https://speed.cloudflare.com/__up";
const DOWNLOAD_CHUNK_BYTES: u64 = 250_000_000;
const UPLOAD_CHUNK_BYTES: usize = 50_000_000;
const SAMPLE_INTERVAL: Duration = Duration::from_millis(200);
const LOADED_LATENCY_INTERVAL: Duration = Duration::from_millis(400);
const PHASE_COOLDOWN: Duration = Duration::from_millis(750);
const MAX_RATE_LIMIT_RETRIES: usize = 3;
const RATE_LIMIT_BASE_DELAY_MS: u64 = 400;
const MAX_RATE_LIMIT_DELAY: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct CloudflareEngine {
    client: Client,
    config: EngineConfig,
}

impl CloudflareEngine {
    pub fn new(config: EngineConfig) -> Result<Self> {
        let client = Client::builder()
            .user_agent(concat!("speedtest-cli/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .pool_max_idle_per_host(config.streams.saturating_add(2))
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self { client, config })
    }

    pub async fn run(&self, tx: UnboundedSender<EngineEvent>) -> Result<TestResult> {
        self.emit(&tx, EngineEvent::PhaseChanged(TestPhase::Preparing));
        self.warm_up().await?;

        self.emit(&tx, EngineEvent::PhaseChanged(TestPhase::Latency));
        let (idle_ms, jitter_ms) = self.measure_idle_latency(12).await?;
        self.emit(
            &tx,
            EngineEvent::IdleLatency {
                ping_ms: idle_ms,
                jitter_ms,
            },
        );

        self.emit(&tx, EngineEvent::PhaseChanged(TestPhase::Download));
        let (download, download_loaded_ms) = self.measure_download(&tx).await?;

        sleep(PHASE_COOLDOWN).await;

        self.emit(&tx, EngineEvent::PhaseChanged(TestPhase::Upload));
        let (upload, upload_loaded_ms) = self.measure_upload(&tx).await?;

        let result = TestResult {
            timestamp: Utc::now(),
            backend: "cloudflare".to_string(),
            server: ServerInfo {
                host: "speed.cloudflare.com".to_string(),
                name: "Cloudflare Edge".to_string(),
            },
            latency: LatencyResult {
                idle_ms,
                jitter_ms,
                download_loaded_ms,
                upload_loaded_ms,
                packet_loss_percent: None,
            },
            download,
            upload,
        };

        self.emit(&tx, EngineEvent::PhaseChanged(TestPhase::Complete));
        self.emit(&tx, EngineEvent::Complete(result.clone()));
        Ok(result)
    }

    async fn warm_up(&self) -> Result<()> {
        let mut retries = 0;

        loop {
            let response = self
                .client
                .get(DOWNLOAD_URL)
                .query(&[("bytes", "1000000")])
                .send()
                .await
                .context("warm-up request failed")?;

            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                retries += 1;
                if retries > MAX_RATE_LIMIT_RETRIES {
                    return Err(anyhow!(
                        "Cloudflare rate limited the warm-up request; wait a moment and try again"
                    ));
                }
                sleep(rate_limit_delay(&response, retries)).await;
                continue;
            }

            let response = response
                .error_for_status()
                .context("warm-up endpoint returned an error")?;
            let _ = response.bytes().await.context("warm-up body failed")?;
            return Ok(());
        }
    }

    async fn measure_idle_latency(&self, count: usize) -> Result<(f64, f64)> {
        let mut samples = Vec::with_capacity(count);

        for _ in 0..count {
            samples.push(self.single_latency_sample().await?);
            sleep(Duration::from_millis(75)).await;
        }

        let ping = median(&samples).unwrap_or(0.0);
        let jitter = mean_consecutive_delta(&samples);
        Ok((ping, jitter))
    }

    async fn single_latency_sample(&self) -> Result<f64> {
        let mut retries = 0;

        loop {
            let started = Instant::now();
            let response = self
                .client
                .get(DOWNLOAD_URL)
                .query(&[("bytes", "0")])
                .header("cache-control", "no-cache")
                .send()
                .await
                .context("latency request failed")?;

            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                retries += 1;
                if retries > MAX_RATE_LIMIT_RETRIES {
                    return Err(anyhow!(
                        "Cloudflare rate limited latency measurements; wait a moment and try again"
                    ));
                }
                sleep(rate_limit_delay(&response, retries)).await;
                continue;
            }

            let response = response
                .error_for_status()
                .context("latency endpoint returned an error")?;
            let _ = response.bytes().await.context("latency body failed")?;
            return Ok(started.elapsed().as_secs_f64() * 1000.0);
        }
    }

    async fn measure_download(
        &self,
        tx: &UnboundedSender<EngineEvent>,
    ) -> Result<(ThroughputResult, Option<f64>)> {
        let total = Arc::new(AtomicU64::new(0));
        let started = Instant::now();
        let deadline = started + self.config.phase_duration;
        let mut workers = JoinSet::new();

        for _ in 0..self.config.streams {
            let client = self.client.clone();
            let total = Arc::clone(&total);
            workers.spawn(async move { download_worker(client, total, deadline).await });
        }

        let loaded_client = self.client.clone();
        let loaded =
            tokio::spawn(async move { measure_loaded_latency(loaded_client, deadline).await });

        self.sample_transfer(TestPhase::Download, Arc::clone(&total), deadline, tx)
            .await;

        while let Some(worker) = workers.join_next().await {
            worker.context("download worker panicked")??;
        }

        let loaded_latency = loaded.await.context("loaded-latency task panicked")??;
        let loaded_median = median(&loaded_latency);
        let elapsed = self.config.phase_duration.as_secs_f64();
        let bytes = total.load(Ordering::Relaxed);

        if bytes == 0 {
            return Err(anyhow!(
                "Cloudflare did not deliver any download data; its public speed-test endpoint may be rate limiting this client"
            ));
        }

        let result = ThroughputResult {
            mbps: mbps(bytes, elapsed),
            bytes,
            seconds: elapsed,
        };

        if let Some(ms) = loaded_median {
            self.emit(
                tx,
                EngineEvent::LoadedLatency {
                    phase: TestPhase::Download,
                    ms,
                },
            );
        }

        Ok((result, loaded_median))
    }

    async fn measure_upload(
        &self,
        tx: &UnboundedSender<EngineEvent>,
    ) -> Result<(ThroughputResult, Option<f64>)> {
        let total = Arc::new(AtomicU64::new(0));
        let payload = Bytes::from(vec![0_u8; UPLOAD_CHUNK_BYTES]);
        let started = Instant::now();
        let deadline = started + self.config.phase_duration;
        let mut workers = JoinSet::new();

        for _ in 0..self.config.streams {
            let client = self.client.clone();
            let total = Arc::clone(&total);
            let payload = payload.clone();
            workers.spawn(async move { upload_worker(client, total, payload, deadline).await });
        }

        let loaded_client = self.client.clone();
        let loaded =
            tokio::spawn(async move { measure_loaded_latency(loaded_client, deadline).await });

        self.sample_transfer(TestPhase::Upload, Arc::clone(&total), deadline, tx)
            .await;

        while let Some(worker) = workers.join_next().await {
            worker.context("upload worker panicked")??;
        }

        let loaded_latency = loaded.await.context("loaded-latency task panicked")??;
        let loaded_median = median(&loaded_latency);
        let elapsed = self.config.phase_duration.as_secs_f64();
        let bytes = total.load(Ordering::Relaxed);

        if bytes == 0 {
            return Err(anyhow!(
                "Cloudflare did not accept any upload data; its public speed-test endpoint may be rate limiting this client"
            ));
        }

        let result = ThroughputResult {
            mbps: mbps(bytes, elapsed),
            bytes,
            seconds: elapsed,
        };

        if let Some(ms) = loaded_median {
            self.emit(
                tx,
                EngineEvent::LoadedLatency {
                    phase: TestPhase::Upload,
                    ms,
                },
            );
        }

        Ok((result, loaded_median))
    }

    async fn sample_transfer(
        &self,
        phase: TestPhase,
        total: Arc<AtomicU64>,
        deadline: Instant,
        tx: &UnboundedSender<EngineEvent>,
    ) {
        let mut interval = tokio::time::interval(SAMPLE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut previous_bytes = 0_u64;
        let mut previous_at = Instant::now();

        loop {
            interval.tick().await;
            let now = Instant::now();
            let current = total.load(Ordering::Relaxed);
            let delta_bytes = current.saturating_sub(previous_bytes);
            let delta_seconds = now.duration_since(previous_at).as_secs_f64();

            if delta_seconds > 0.0 {
                self.emit(
                    tx,
                    EngineEvent::ThroughputSample {
                        phase,
                        mbps: mbps(delta_bytes, delta_seconds),
                    },
                );
            }

            previous_bytes = current;
            previous_at = now;

            if now >= deadline {
                break;
            }
        }
    }

    fn emit(&self, tx: &UnboundedSender<EngineEvent>, event: EngineEvent) {
        let _ = tx.send(event);
    }
}

async fn download_worker(client: Client, total: Arc<AtomicU64>, deadline: Instant) -> Result<()> {
    let mut rate_limit_retries = 0;

    while Instant::now() < deadline {
        let response = tokio::select! {
            result = client
                .get(DOWNLOAD_URL)
                .query(&[("bytes", DOWNLOAD_CHUNK_BYTES)])
                .send() => result.context("download request failed")?,
            _ = sleep_until(deadline) => break,
        };

        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            rate_limit_retries += 1;
            if rate_limit_retries > MAX_RATE_LIMIT_RETRIES {
                break;
            }
            if !sleep_before_deadline(rate_limit_delay(&response, rate_limit_retries), deadline).await
            {
                break;
            }
            continue;
        }

        rate_limit_retries = 0;
        let response = response
            .error_for_status()
            .context("download endpoint returned an error")?;

        let mut body = response.bytes_stream();
        loop {
            let next = tokio::select! {
                chunk = body.next() => chunk,
                _ = sleep_until(deadline) => None,
            };

            match next {
                Some(Ok(chunk)) => {
                    total.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                }
                Some(Err(error)) => return Err(error).context("download stream failed"),
                None => break,
            }

            if Instant::now() >= deadline {
                break;
            }
        }
    }

    Ok(())
}

async fn upload_worker(
    client: Client,
    total: Arc<AtomicU64>,
    payload: Bytes,
    deadline: Instant,
) -> Result<()> {
    let mut rate_limit_retries = 0;

    while Instant::now() < deadline {
        let response = tokio::select! {
            result = client
                .post(UPLOAD_URL)
                .header("content-type", "application/octet-stream")
                .body(payload.clone())
                .send() => Some(result.context("upload request failed")?),
            _ = sleep_until(deadline) => None,
        };

        let Some(response) = response else {
            break;
        };

        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            rate_limit_retries += 1;
            if rate_limit_retries > MAX_RATE_LIMIT_RETRIES {
                break;
            }
            if !sleep_before_deadline(rate_limit_delay(&response, rate_limit_retries), deadline).await
            {
                break;
            }
            continue;
        }

        rate_limit_retries = 0;
        response
            .error_for_status()
            .context("upload endpoint returned an error")?;
        total.fetch_add(payload.len() as u64, Ordering::Relaxed);
    }

    Ok(())
}

async fn measure_loaded_latency(client: Client, deadline: Instant) -> Result<Vec<f64>> {
    let mut samples = Vec::new();
    let mut rate_limit_retries = 0;

    while Instant::now() < deadline {
        let started = Instant::now();
        let request = client
            .get(DOWNLOAD_URL)
            .query(&[("bytes", "0"), ("during", "load")])
            .header("cache-control", "no-cache")
            .send();

        let response = tokio::select! {
            result = request => Some(result.context("loaded latency request failed")?),
            _ = sleep_until(deadline) => None,
        };

        let Some(response) = response else {
            break;
        };

        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            rate_limit_retries += 1;
            if rate_limit_retries > MAX_RATE_LIMIT_RETRIES {
                break;
            }
            if !sleep_before_deadline(rate_limit_delay(&response, rate_limit_retries), deadline).await
            {
                break;
            }
            continue;
        }

        rate_limit_retries = 0;
        response
            .error_for_status()
            .context("loaded latency endpoint returned an error")?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);

        if !sleep_before_deadline(LOADED_LATENCY_INTERVAL, deadline).await {
            break;
        }
    }

    Ok(samples)
}

async fn sleep_before_deadline(delay: Duration, deadline: Instant) -> bool {
    let now = Instant::now();
    if now >= deadline {
        return false;
    }

    let remaining = deadline.saturating_duration_since(now);
    sleep(delay.min(remaining)).await;
    Instant::now() < deadline
}

fn rate_limit_delay(response: &Response, attempt: usize) -> Duration {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| fallback_rate_limit_delay(attempt))
        .min(MAX_RATE_LIMIT_DELAY)
}

fn fallback_rate_limit_delay(attempt: usize) -> Duration {
    let exponent = attempt.saturating_sub(1).min(3) as u32;
    Duration::from_millis(RATE_LIMIT_BASE_DELAY_MS * 2_u64.pow(exponent))
}

fn mbps(bytes: u64, seconds: f64) -> f64 {
    if seconds <= f64::EPSILON {
        return 0.0;
    }
    bytes as f64 * 8.0 / seconds / 1_000_000.0
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;

    if sorted.len().is_multiple_of(2) {
        Some((sorted[middle - 1] + sorted[middle]) / 2.0)
    } else {
        Some(sorted[middle])
    }
}

fn mean_consecutive_delta(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }

    let total: f64 = values
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .sum();
    total / (values.len() - 1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_mbps() {
        assert!((mbps(125_000_000, 1.0) - 1000.0).abs() < 0.001);
    }

    #[test]
    fn calculates_median_for_odd_and_even_sets() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
    }

    #[test]
    fn calculates_jitter_as_mean_delta() {
        let jitter = mean_consecutive_delta(&[10.0, 12.0, 11.0, 15.0]);
        assert!((jitter - (7.0 / 3.0)).abs() < 0.001);
    }

    #[test]
    fn rate_limit_backoff_grows_and_caps() {
        assert_eq!(fallback_rate_limit_delay(1), Duration::from_millis(400));
        assert_eq!(fallback_rate_limit_delay(2), Duration::from_millis(800));
        assert_eq!(fallback_rate_limit_delay(3), Duration::from_millis(1600));
        assert_eq!(fallback_rate_limit_delay(4), Duration::from_millis(3200));
        assert_eq!(fallback_rate_limit_delay(8), Duration::from_millis(3200));
    }
}
