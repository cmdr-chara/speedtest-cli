use std::{
    collections::VecDeque,
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
use reqwest::{
    header::{CONTENT_LENGTH, RETRY_AFTER},
    Body, Client, Response, StatusCode,
};
use tokio::{
    sync::mpsc::UnboundedSender,
    task::JoinSet,
    time::{sleep, sleep_until, Instant},
};

use crate::{
    analysis,
    engine::{EngineConfig, EngineEvent},
    model::{ServerInfo, TestPhase, TestResult, ThroughputResult},
};

const DOWNLOAD_URL: &str = "https://speed.cloudflare.com/__down";
const UPLOAD_URL: &str = "https://speed.cloudflare.com/__up";
const DOWNLOAD_LADDER: [u64; 4] = [95_000_000, 25_000_000, 10_000_000, 1_000_000];
const DOWNLOAD_START_INDEX: usize = 1;
const UPLOAD_START_BYTES: usize = 25_000_000;
const UPLOAD_MIN_BYTES: usize = 1_000_000;
const UPLOAD_BODY_CHUNK_BYTES: usize = 64 * 1024;
const SAMPLE_INTERVAL: Duration = Duration::from_millis(200);
const THROUGHPUT_WINDOW: Duration = Duration::from_millis(900);
const DISPLAY_WARMUP: Duration = Duration::from_millis(400);
const LOADED_LATENCY_INTERVAL: Duration = Duration::from_millis(400);
const PHASE_COOLDOWN: Duration = Duration::from_millis(500);
const IDLE_LATENCY_SAMPLES: usize = 24;
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
            .timeout(Duration::from_secs(25))
            .pool_max_idle_per_host(config.streams.saturating_add(4))
            .build()
            .context("failed to build Cloudflare HTTP client")?;
        Ok(Self { client, config })
    }

    pub async fn run(&self, tx: UnboundedSender<EngineEvent>) -> Result<TestResult> {
        self.emit(&tx, EngineEvent::PhaseChanged(TestPhase::Preparing));
        self.warm_up().await?;

        self.emit(&tx, EngineEvent::PhaseChanged(TestPhase::Latency));
        let idle_samples = self.measure_idle_latency(IDLE_LATENCY_SAMPLES).await?;
        let preview = analysis::summarize_latency(&idle_samples, &[], &[], None);
        self.emit(
            &tx,
            EngineEvent::IdleLatency {
                ping_ms: preview.idle_ms,
                jitter_ms: preview.jitter_ms,
            },
        );

        self.emit(&tx, EngineEvent::PhaseChanged(TestPhase::Download));
        let (download, download_loaded) = self.measure_download(&tx).await?;
        sleep(PHASE_COOLDOWN).await;

        self.emit(&tx, EngineEvent::PhaseChanged(TestPhase::Upload));
        let (upload, upload_loaded) = self.measure_upload(&tx).await?;

        let latency =
            analysis::summarize_latency(&idle_samples, &download_loaded, &upload_loaded, None);
        let network_analysis = analysis::build_network_analysis(
            &idle_samples,
            &download_loaded,
            &upload_loaded,
            &latency,
            &download,
            &upload,
        );
        let result = TestResult {
            timestamp: Utc::now(),
            backend: "cloudflare".to_string(),
            server: ServerInfo {
                host: "speed.cloudflare.com".to_string(),
                name: "Cloudflare Edge (adaptive)".to_string(),
            },
            latency,
            download,
            upload,
            analysis: Some(network_analysis),
        };
        self.emit(&tx, EngineEvent::PhaseChanged(TestPhase::Complete));
        self.emit(&tx, EngineEvent::Complete(result.clone()));
        Ok(result)
    }

    async fn warm_up(&self) -> Result<()> {
        for bytes in [1_000_000_u64, 100_000, 1] {
            let mut rate_limit_retries = 0_usize;
            loop {
                let response = self
                    .client
                    .get(DOWNLOAD_URL)
                    .query(&[("bytes", bytes), ("warmup", cache_buster())])
                    .header("cache-control", "no-store")
                    .send()
                    .await
                    .context("Cloudflare warm-up request failed")?;
                if response.status().is_success() {
                    let _ = response
                        .bytes()
                        .await
                        .context("Cloudflare warm-up body failed")?;
                    return Ok(());
                }
                if response.status() == StatusCode::TOO_MANY_REQUESTS {
                    rate_limit_retries += 1;
                    if rate_limit_retries > MAX_RATE_LIMIT_RETRIES {
                        return Err(anyhow!(
                            "Cloudflare rate limited warm-up probes after {MAX_RATE_LIMIT_RETRIES} retries"
                        ));
                    }
                    sleep(rate_limit_delay(&response, rate_limit_retries)).await;
                    continue;
                }
                if is_size_rejection(response.status()) {
                    break;
                }
                response
                    .error_for_status()
                    .context("Cloudflare warm-up endpoint returned an error")?;
            }
        }
        Err(anyhow!(
            "Cloudflare rejected warm-up probes down to 1 byte; try `--backend librespeed` or `speedtest verify`"
        ))
    }

    async fn measure_idle_latency(&self, count: usize) -> Result<Vec<f64>> {
        let mut samples = Vec::with_capacity(count);
        for _ in 0..count {
            samples.push(latency_probe(&self.client).await?);
            sleep(Duration::from_millis(75)).await;
        }
        Ok(samples)
    }

    async fn measure_download(
        &self,
        tx: &UnboundedSender<EngineEvent>,
    ) -> Result<(ThroughputResult, Vec<f64>)> {
        let total = Arc::new(AtomicU64::new(0));
        let deadline = Instant::now() + self.config.phase_duration;
        let mut workers = JoinSet::new();
        for _ in 0..self.config.streams.max(1) {
            workers.spawn(download_worker(
                self.client.clone(),
                Arc::clone(&total),
                deadline,
            ));
        }
        let loaded = tokio::spawn(measure_loaded_latency(self.client.clone(), deadline));
        self.sample_transfer(TestPhase::Download, Arc::clone(&total), deadline, tx)
            .await;
        while let Some(result) = workers.join_next().await {
            result.context("Cloudflare download worker panicked")??;
        }
        let loaded_samples = loaded.await.context("loaded-latency task panicked")?;
        let bytes = total.load(Ordering::Relaxed);
        if bytes == 0 {
            return Err(anyhow!(
                "Cloudflare delivered no download data; try `--backend librespeed`"
            ));
        }
        emit_loaded(&loaded_samples, TestPhase::Download, tx);
        Ok((
            throughput(bytes, self.config.phase_duration),
            loaded_samples,
        ))
    }

    async fn measure_upload(
        &self,
        tx: &UnboundedSender<EngineEvent>,
    ) -> Result<(ThroughputResult, Vec<f64>)> {
        let total = Arc::new(AtomicU64::new(0));
        let deadline = Instant::now() + self.config.phase_duration;
        let mut workers = JoinSet::new();
        for _ in 0..self.config.streams.max(1) {
            workers.spawn(upload_worker(
                self.client.clone(),
                Arc::clone(&total),
                UPLOAD_START_BYTES,
                deadline,
            ));
        }
        let loaded = tokio::spawn(measure_loaded_latency(self.client.clone(), deadline));
        self.sample_transfer(TestPhase::Upload, Arc::clone(&total), deadline, tx)
            .await;
        while let Some(result) = workers.join_next().await {
            result.context("Cloudflare upload worker panicked")??;
        }
        let loaded_samples = loaded.await.context("loaded-latency task panicked")?;
        let bytes = total.load(Ordering::Relaxed);
        if bytes == 0 {
            return Err(anyhow!(
                "Cloudflare accepted no upload data; try `--backend librespeed`"
            ));
        }
        emit_loaded(&loaded_samples, TestPhase::Upload, tx);
        Ok((
            throughput(bytes, self.config.phase_duration),
            loaded_samples,
        ))
    }

    async fn sample_transfer(
        &self,
        phase: TestPhase,
        total: Arc<AtomicU64>,
        deadline: Instant,
        tx: &UnboundedSender<EngineEvent>,
    ) {
        let started = Instant::now();
        let mut interval = tokio::time::interval(SAMPLE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut history = VecDeque::with_capacity(8);

        loop {
            interval.tick().await;
            let now = Instant::now();
            let current = total.load(Ordering::Relaxed);
            history.push_back((now, current));

            while history.len() > 2 {
                let second = history.get(1).expect("history contains two samples");
                if now.duration_since(second.0) >= THROUGHPUT_WINDOW {
                    history.pop_front();
                } else {
                    break;
                }
            }

            if now.duration_since(started) >= DISPLAY_WARMUP {
                if let Some((window_start, window_bytes)) = history.front().copied() {
                    let elapsed = now.duration_since(window_start).as_secs_f64();
                    if elapsed >= SAMPLE_INTERVAL.as_secs_f64() {
                        self.emit(
                            tx,
                            EngineEvent::ThroughputSample {
                                phase,
                                mbps: mbps(current.saturating_sub(window_bytes), elapsed),
                            },
                        );
                    }
                }
            }

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
    let mut chunk_index = DOWNLOAD_START_INDEX;
    let mut floor_index = 0_usize;
    let mut rate_limit_retries = 0_usize;

    while Instant::now() < deadline {
        let bytes = DOWNLOAD_LADDER[chunk_index];
        let request_started = Instant::now();
        let response = tokio::select! {
            response = client
                .get(DOWNLOAD_URL)
                .query(&[("bytes", bytes), ("r", cache_buster())])
                .header("accept", "application/octet-stream")
                .header("cache-control", "no-store")
                .send() => response.context("Cloudflare download request failed")?,
            _ = sleep_until(deadline) => break,
        };

        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            rate_limit_retries += 1;
            if rate_limit_retries > MAX_RATE_LIMIT_RETRIES
                || !sleep_before_deadline(rate_limit_delay(&response, rate_limit_retries), deadline)
                    .await
            {
                break;
            }
            continue;
        }
        if is_size_rejection(response.status()) {
            if chunk_index + 1 < DOWNLOAD_LADDER.len() {
                chunk_index += 1;
                floor_index = floor_index.max(chunk_index);
                continue;
            }
            return Err(anyhow!(
                "Cloudflare rejected download probes down to {} byte(s) with {}; use `--backend librespeed`",
                bytes,
                response.status()
            ));
        }

        rate_limit_retries = 0;
        let response = response
            .error_for_status()
            .context("Cloudflare download endpoint returned an error")?;
        let mut body = response.bytes_stream();
        let mut completed = true;
        loop {
            let next = tokio::select! {
                chunk = body.next() => chunk,
                _ = sleep_until(deadline) => {
                    completed = false;
                    None
                },
            };
            match next {
                Some(Ok(chunk)) => {
                    total.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                }
                Some(Err(error)) => return Err(error).context("Cloudflare download stream failed"),
                None => break,
            }
            if Instant::now() >= deadline {
                completed = false;
                break;
            }
        }

        if completed
            && request_started.elapsed() < Duration::from_millis(700)
            && chunk_index > floor_index
        {
            chunk_index -= 1;
        }
    }
    Ok(())
}

async fn upload_worker(
    client: Client,
    total: Arc<AtomicU64>,
    initial_payload_len: usize,
    deadline: Instant,
) -> Result<()> {
    let mut payload_len = initial_payload_len;
    let mut rate_limit_retries = 0_usize;
    while Instant::now() < deadline {
        let response = tokio::select! {
            response = client
                .post(UPLOAD_URL)
                .query(&[("r", cache_buster())])
                .header("content-type", "application/octet-stream")
                .header("cache-control", "no-store")
                .header(CONTENT_LENGTH, payload_len.to_string())
                .body(counted_upload_body(Arc::clone(&total), payload_len))
                .send() => response.context("Cloudflare upload request failed")?,
            _ = sleep_until(deadline) => break,
        };

        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            rate_limit_retries += 1;
            if rate_limit_retries > MAX_RATE_LIMIT_RETRIES
                || !sleep_before_deadline(rate_limit_delay(&response, rate_limit_retries), deadline)
                    .await
            {
                break;
            }
            continue;
        }
        if is_size_rejection(response.status()) && payload_len > UPLOAD_MIN_BYTES {
            payload_len = (payload_len / 2).max(UPLOAD_MIN_BYTES);
            continue;
        }

        rate_limit_retries = 0;
        response
            .error_for_status()
            .context("Cloudflare upload endpoint returned an error")?;
    }
    Ok(())
}

fn counted_upload_body(total: Arc<AtomicU64>, payload_len: usize) -> Body {
    let chunk = Bytes::from(vec![0_u8; UPLOAD_BODY_CHUNK_BYTES]);
    let stream = futures_util::stream::unfold(
        (payload_len, chunk, total),
        |(remaining, chunk, total)| async move {
            if remaining == 0 {
                return None;
            }
            let len = remaining.min(chunk.len());
            total.fetch_add(len as u64, Ordering::Relaxed);
            let item = Ok::<Bytes, std::io::Error>(chunk.slice(..len));
            Some((item, (remaining - len, chunk, total)))
        },
    );
    Body::wrap_stream(stream)
}

async fn latency_probe(client: &Client) -> Result<f64> {
    let mut rate_limit_retries = 0_usize;
    for bytes in [0_u64, 1] {
        loop {
            let started = Instant::now();
            let response = client
                .get(DOWNLOAD_URL)
                .query(&[("bytes", bytes), ("r", cache_buster())])
                .header("cache-control", "no-store")
                .send()
                .await
                .context("Cloudflare latency request failed")?;
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                rate_limit_retries += 1;
                if rate_limit_retries > MAX_RATE_LIMIT_RETRIES {
                    return Err(anyhow!("Cloudflare rate limited latency probes"));
                }
                sleep(rate_limit_delay(&response, rate_limit_retries)).await;
                continue;
            }
            if is_size_rejection(response.status()) && bytes == 0 {
                break;
            }
            let response = response
                .error_for_status()
                .context("Cloudflare latency endpoint returned an error")?;
            let _ = response
                .bytes()
                .await
                .context("Cloudflare latency body failed")?;
            return Ok(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    Err(anyhow!(
        "Cloudflare rejected both zero-byte and one-byte latency probes"
    ))
}

async fn measure_loaded_latency(client: Client, deadline: Instant) -> Vec<f64> {
    let mut samples = Vec::new();
    while Instant::now() < deadline {
        let now = Instant::now();
        let remaining = deadline.saturating_duration_since(now);
        if remaining.is_zero() {
            break;
        }
        let probe = tokio::time::timeout(
            remaining.min(Duration::from_secs(3)),
            latency_probe(&client),
        );
        if let Ok(Ok(ms)) = probe.await {
            samples.push(ms);
        }
        if !sleep_before_deadline(LOADED_LATENCY_INTERVAL, deadline).await {
            break;
        }
    }
    samples
}

fn is_size_rejection(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::FORBIDDEN | StatusCode::PAYLOAD_TOO_LARGE | StatusCode::BAD_REQUEST
    )
}

async fn sleep_before_deadline(delay: Duration, deadline: Instant) -> bool {
    let now = Instant::now();
    if now >= deadline {
        return false;
    }
    sleep(delay.min(deadline.saturating_duration_since(now))).await;
    Instant::now() < deadline
}

fn rate_limit_delay(response: &Response, attempt: usize) -> Duration {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| {
            let exponent = attempt.saturating_sub(1).min(3) as u32;
            Duration::from_millis(RATE_LIMIT_BASE_DELAY_MS * 2_u64.pow(exponent))
        })
        .min(MAX_RATE_LIMIT_DELAY)
}

fn emit_loaded(samples: &[f64], phase: TestPhase, tx: &UnboundedSender<EngineEvent>) {
    if let Some(ms) = analysis::distribution(samples).map(|stats| stats.median_ms) {
        let _ = tx.send(EngineEvent::LoadedLatency { phase, ms });
    }
}

fn throughput(bytes: u64, duration: Duration) -> ThroughputResult {
    let seconds = duration.as_secs_f64();
    ThroughputResult {
        mbps: mbps(bytes, seconds),
        bytes,
        seconds,
    }
}

fn mbps(bytes: u64, seconds: f64) -> f64 {
    if seconds <= f64::EPSILON {
        0.0
    } else {
        bytes as f64 * 8.0 / seconds / 1_000_000.0
    }
}

fn cache_buster() -> u64 {
    Utc::now().timestamp_micros().unsigned_abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_below_previous_forbidden_250mb_request() {
        assert!(DOWNLOAD_LADDER[DOWNLOAD_START_INDEX] < 250_000_000);
        assert_eq!(DOWNLOAD_LADDER[DOWNLOAD_START_INDEX], 25_000_000);
        assert_eq!(*DOWNLOAD_LADDER.last().unwrap(), 1_000_000);
    }

    #[test]
    fn size_rejections_are_downshift_candidates() {
        assert!(is_size_rejection(StatusCode::FORBIDDEN));
        assert!(is_size_rejection(StatusCode::PAYLOAD_TOO_LARGE));
        assert!(!is_size_rejection(StatusCode::TOO_MANY_REQUESTS));
    }

    #[test]
    fn decimal_mbps_conversion_is_correct() {
        assert!((mbps(125_000_000, 1.0) - 1000.0).abs() < 0.001);
    }
}
