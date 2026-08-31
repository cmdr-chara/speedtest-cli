use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    task::JoinSet,
    time::{sleep, sleep_until, timeout, Instant},
};

use crate::{
    analysis,
    model::{ServerInfo, TestResult, ThroughputResult},
};

const MAGIC: &[u8; 4] = b"NSL1";
const OP_PING: u8 = 0;
const OP_DOWNLOAD: u8 = 1;
const OP_UPLOAD: u8 = 2;
const IO_CHUNK: usize = 64 * 1024;
const MAX_CONCURRENT_CONNECTIONS: usize = 64;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
const SERVER_TRANSFER_TIMEOUT: Duration = Duration::from_secs(45);
const PING_TIMEOUT: Duration = Duration::from_secs(2);
const LOADED_PING_INTERVAL: Duration = Duration::from_millis(250);
const IDLE_SAMPLES: usize = 20;

#[derive(Debug, Clone, Copy)]
pub struct LanConfig {
    pub streams: usize,
    pub phase_duration: Duration,
}

pub async fn serve(bind: SocketAddr) -> Result<()> {
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind LAN speed-test server to {bind}"))?;
    let listening = listener.local_addr().unwrap_or(bind);
    println!("LAN SPEEDTEST SERVER");
    println!("  Listening: {listening}");
    println!(
        "  Client:    speedtest lan <this-host>:{}",
        listening.port()
    );
    println!("  Stop with Ctrl+C.");
    serve_listener(listener).await
}

async fn serve_listener(listener: TcpListener) -> Result<()> {
    let connection_slots = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .context("LAN server accept failed")?;
        let Ok(permit) = Arc::clone(&connection_slots).try_acquire_owned() else {
            drop(stream);
            continue;
        };
        tokio::spawn(async move {
            let _permit = permit;
            let _ = handle_connection(stream).await;
        });
    }
}

pub async fn run(server: SocketAddr, config: LanConfig) -> Result<TestResult> {
    let mut idle_samples = Vec::with_capacity(IDLE_SAMPLES);
    for _ in 0..IDLE_SAMPLES {
        idle_samples.push(ping_once(server).await?);
        sleep(Duration::from_millis(50)).await;
    }

    let (download, download_loaded) = measure_download(server, config).await?;
    sleep(Duration::from_millis(250)).await;
    let (upload, upload_loaded) = measure_upload(server, config).await?;

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

    Ok(TestResult {
        timestamp: Utc::now(),
        backend: "lan".to_string(),
        server: ServerInfo {
            host: server.to_string(),
            name: "Self-hosted LAN endpoint".to_string(),
        },
        latency,
        download,
        upload,
        analysis: Some(network_analysis),
    })
}

async fn handle_connection(mut stream: TcpStream) -> Result<()> {
    handle_connection_with_timeouts(&mut stream, HANDSHAKE_TIMEOUT, SERVER_TRANSFER_TIMEOUT).await
}

async fn handle_connection_with_timeouts(
    stream: &mut TcpStream,
    handshake_timeout: Duration,
    transfer_timeout: Duration,
) -> Result<()> {
    stream
        .set_nodelay(true)
        .context("failed to enable TCP_NODELAY")?;
    let mut header = [0_u8; 5];
    timeout(handshake_timeout, stream.read_exact(&mut header))
        .await
        .context("LAN protocol header timed out")?
        .context("LAN client closed before protocol header")?;
    if &header[..4] != MAGIC {
        return Err(anyhow!("invalid LAN speed-test protocol magic"));
    }

    timeout(transfer_timeout, handle_request(stream, header[4]))
        .await
        .context("LAN connection exceeded transfer time limit")??;
    Ok(())
}

async fn handle_request(stream: &mut TcpStream, opcode: u8) -> Result<()> {
    match opcode {
        OP_PING => {
            stream.write_all(MAGIC).await?;
            stream.write_u8(OP_PING).await?;
            stream.flush().await?;
        }
        OP_DOWNLOAD => {
            let chunk = vec![0x5a_u8; IO_CHUNK];
            loop {
                if stream.write_all(&chunk).await.is_err() {
                    break;
                }
            }
        }
        OP_UPLOAD => {
            let mut total = 0_u64;
            let mut buffer = vec![0_u8; IO_CHUNK];
            loop {
                match stream.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(count) => total = total.saturating_add(count as u64),
                    Err(_) => break,
                }
            }
            let _ = stream.write_u64(total).await;
            let _ = stream.flush().await;
        }
        opcode => return Err(anyhow!("unknown LAN speed-test opcode {opcode}")),
    }
    Ok(())
}

async fn ping_once(server: SocketAddr) -> Result<f64> {
    let started = Instant::now();
    let mut stream = timeout(PING_TIMEOUT, TcpStream::connect(server))
        .await
        .context("LAN ping connection timed out")??;
    stream.set_nodelay(true)?;
    let mut response = [0_u8; 5];
    timeout(PING_TIMEOUT, async {
        stream.write_all(MAGIC).await?;
        stream.write_u8(OP_PING).await?;
        stream.flush().await?;
        stream.read_exact(&mut response).await?;
        Result::<(), std::io::Error>::Ok(())
    })
    .await
    .context("LAN echo timed out")??;
    if &response[..4] != MAGIC {
        return Err(anyhow!("invalid LAN ping response magic"));
    }
    if response[4] != OP_PING {
        return Err(anyhow!("invalid LAN ping response opcode {}", response[4]));
    }
    Ok(started.elapsed().as_secs_f64() * 1000.0)
}

async fn measure_download(
    server: SocketAddr,
    config: LanConfig,
) -> Result<(ThroughputResult, Vec<f64>)> {
    let total = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    let deadline = started + config.phase_duration;
    let mut workers = JoinSet::new();

    for _ in 0..config.streams.max(1) {
        let total = Arc::clone(&total);
        workers.spawn(download_worker(server, total, deadline));
    }
    let loaded = tokio::spawn(measure_loaded_latency(server, deadline));

    while let Some(result) = workers.join_next().await {
        result.context("LAN download worker panicked")??;
    }
    let loaded_samples = loaded.await.context("LAN loaded-latency task panicked")?;
    let bytes = total.load(Ordering::Relaxed);
    if bytes == 0 {
        return Err(anyhow!("LAN server delivered no download data"));
    }

    Ok((
        ThroughputResult {
            mbps: mbps(bytes, config.phase_duration.as_secs_f64()),
            bytes,
            seconds: config.phase_duration.as_secs_f64(),
        },
        loaded_samples,
    ))
}

async fn download_worker(
    server: SocketAddr,
    total: Arc<AtomicU64>,
    deadline: Instant,
) -> Result<()> {
    let mut stream = TcpStream::connect(server)
        .await
        .with_context(|| format!("failed to connect to LAN server {server}"))?;
    stream.set_nodelay(true)?;
    stream.write_all(MAGIC).await?;
    stream.write_u8(OP_DOWNLOAD).await?;
    stream.flush().await?;
    let mut buffer = vec![0_u8; IO_CHUNK];

    while Instant::now() < deadline {
        let read = tokio::select! {
            value = stream.read(&mut buffer) => value?,
            _ = sleep_until(deadline) => break,
        };
        if read == 0 {
            break;
        }
        total.fetch_add(read as u64, Ordering::Relaxed);
    }
    Ok(())
}

async fn measure_upload(
    server: SocketAddr,
    config: LanConfig,
) -> Result<(ThroughputResult, Vec<f64>)> {
    let total = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    let deadline = started + config.phase_duration;
    let mut workers = JoinSet::new();

    for _ in 0..config.streams.max(1) {
        let total = Arc::clone(&total);
        workers.spawn(upload_worker(server, total, deadline));
    }
    let loaded = tokio::spawn(measure_loaded_latency(server, deadline));

    while let Some(result) = workers.join_next().await {
        result.context("LAN upload worker panicked")??;
    }
    let loaded_samples = loaded.await.context("LAN loaded-latency task panicked")?;
    let bytes = total.load(Ordering::Relaxed);
    if bytes == 0 {
        return Err(anyhow!("LAN server accepted no upload data"));
    }

    Ok((
        ThroughputResult {
            mbps: mbps(bytes, config.phase_duration.as_secs_f64()),
            bytes,
            seconds: config.phase_duration.as_secs_f64(),
        },
        loaded_samples,
    ))
}

async fn upload_worker(server: SocketAddr, total: Arc<AtomicU64>, deadline: Instant) -> Result<()> {
    let mut stream = TcpStream::connect(server)
        .await
        .with_context(|| format!("failed to connect to LAN server {server}"))?;
    stream.set_nodelay(true)?;
    stream.write_all(MAGIC).await?;
    stream.write_u8(OP_UPLOAD).await?;
    stream.flush().await?;
    let buffer = vec![0xa5_u8; IO_CHUNK];
    let mut written = 0_u64;

    while Instant::now() < deadline {
        let write = tokio::select! {
            value = stream.write(&buffer) => value?,
            _ = sleep_until(deadline) => break,
        };
        if write == 0 {
            break;
        }
        written = written
            .checked_add(write as u64)
            .ok_or_else(|| anyhow!("LAN upload byte counter overflowed"))?;
    }

    stream.shutdown().await?;
    let acknowledged = timeout(Duration::from_secs(3), stream.read_u64())
        .await
        .context("LAN upload acknowledgement timed out")??;
    if acknowledged > written {
        return Err(anyhow!(
            "LAN server acknowledged {acknowledged} upload bytes, but this worker sent only {written}"
        ));
    }
    total.fetch_add(acknowledged, Ordering::Relaxed);
    Ok(())
}

async fn measure_loaded_latency(server: SocketAddr, deadline: Instant) -> Vec<f64> {
    let mut samples = Vec::new();
    while Instant::now() < deadline {
        if let Ok(ms) = ping_once(server).await {
            samples.push(ms);
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        sleep(LOADED_PING_INTERVAL.min(deadline.saturating_duration_since(now))).await;
    }
    samples
}

fn mbps(bytes: u64, seconds: f64) -> f64 {
    if seconds <= f64::EPSILON {
        return 0.0;
    }
    bytes as f64 * 8.0 / seconds / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_throughput_conversion_is_decimal_mbps() {
        assert!((mbps(125_000_000, 1.0) - 1000.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn lan_ping_round_trip_uses_valid_protocol_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            handle_connection(stream).await
        });

        let latency = ping_once(address).await.unwrap();
        assert!(latency.is_finite());
        server.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn lan_ping_rejects_invalid_magic_and_opcode() {
        for (response, expected_error) in [
            (*b"BAD!\0", "invalid LAN ping response magic"),
            (*b"NSL1\x01", "invalid LAN ping response opcode 1"),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 5];
                stream.read_exact(&mut request).await.unwrap();
                assert_eq!(&request[..4], MAGIC);
                assert_eq!(request[4], OP_PING);
                stream.write_all(&response).await.unwrap();
            });

            let error = ping_once(address).await.unwrap_err();
            assert_eq!(error.to_string(), expected_error);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn lan_upload_rejects_acknowledgement_larger_than_worker_sent() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 5];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request[..4], MAGIC);
            assert_eq!(request[4], OP_UPLOAD);

            let mut received = 0_u64;
            let mut buffer = vec![0_u8; IO_CHUNK];
            loop {
                let count = stream.read(&mut buffer).await.unwrap();
                if count == 0 {
                    break;
                }
                received += count as u64;
            }
            stream.write_u64(received + 1).await.unwrap();
            received
        });

        let total = Arc::new(AtomicU64::new(0));
        let error = upload_worker(
            address,
            Arc::clone(&total),
            Instant::now() + Duration::from_millis(25),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("but this worker sent only"));
        assert_eq!(total.load(Ordering::Relaxed), 0);
        assert!(server.await.unwrap() > 0);
    }

    #[tokio::test]
    async fn lan_stalled_header_is_timed_out() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = TcpStream::connect(address).await.unwrap();
        let (mut stream, _) = listener.accept().await.unwrap();

        let error = handle_connection_with_timeouts(
            &mut stream,
            Duration::from_millis(25),
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("LAN protocol header timed out"));
        drop(client);
    }

    #[tokio::test]
    async fn lan_stalled_transfer_is_timed_out() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(address).await.unwrap();
        let (mut stream, _) = listener.accept().await.unwrap();
        client.write_all(MAGIC).await.unwrap();
        client.write_u8(OP_UPLOAD).await.unwrap();

        let error = handle_connection_with_timeouts(
            &mut stream,
            Duration::from_secs(1),
            Duration::from_millis(25),
        )
        .await
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("LAN connection exceeded transfer time limit"));
    }
}
