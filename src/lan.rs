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
const IO_CHUNK: usize = 1024 * 1024;
const PING_TIMEOUT: Duration = Duration::from_secs(2);
const LOADED_PING_INTERVAL: Duration = Duration::from_millis(250);
const IDLE_SAMPLES: usize = 20;
const MAX_CONNECTIONS: usize = 64;
const SESSION_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Clone, Copy)]
pub struct LanConfig {
    pub streams: usize,
    pub phase_duration: Duration,
}

pub async fn serve(bind: SocketAddr) -> Result<()> {
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind LAN speed-test server to {bind}"))?;
    serve_listener(listener).await
}

/// Run on an already-bound listener, allowing accurate readiness and ephemeral-port tests.
pub async fn serve_listener(listener: TcpListener) -> Result<()> {
    let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.context("LAN server accept failed")?;
                if let Ok(permit) = Arc::clone(&permits).try_acquire_owned() {
                    connections.spawn(async move {
                        let _permit = permit;
                        let _ = timeout(SESSION_TIMEOUT, handle_connection(stream)).await;
                    });
                }
                // When full, drop the connection rather than allocating a task/queue.
            }
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}

pub async fn run(server: SocketAddr, config: LanConfig) -> Result<TestResult> {
    crate::engine::EngineConfig {
        streams: config.streams,
        phase_duration: config.phase_duration,
    }
    .validate()?;
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
    stream
        .set_nodelay(true)
        .context("failed to enable TCP_NODELAY")?;
    let mut header = [0_u8; 5];
    timeout(PING_TIMEOUT, stream.read_exact(&mut header))
        .await
        .context("LAN protocol header timed out")?
        .context("LAN client closed before protocol header")?;
    if &header[..4] != MAGIC {
        return Err(anyhow!("invalid LAN speed-test protocol magic"));
    }

    match header[4] {
        OP_PING => {
            stream.write_all(MAGIC).await?;
            stream.write_u8(OP_PING).await?;
            stream.flush().await?;
        }
        OP_DOWNLOAD => {
            let chunk = vec![0x5a_u8; IO_CHUNK];
            loop {
                if !matches!(
                    timeout(Duration::from_secs(5), stream.write_all(&chunk)).await,
                    Ok(Ok(()))
                ) {
                    break;
                }
            }
        }
        OP_UPLOAD => {
            let mut total = 0_u64;
            let mut buffer = vec![0_u8; IO_CHUNK];
            loop {
                match timeout(Duration::from_secs(5), stream.read(&mut buffer))
                    .await
                    .context("LAN upload idle timeout")?
                {
                    Ok(0) => break,
                    Ok(count) => total = total.saturating_add(count as u64),
                    Err(error) => return Err(error.into()),
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
        .context("LAN ping connection timed out")?
        .context("failed to connect to LAN server; start `speedtest serve --bind <LAN-IP>:9876` on the peer and check its firewall")?;
    stream.set_nodelay(true)?;
    timeout(PING_TIMEOUT, async {
        stream.write_all(MAGIC).await?;
        stream.write_u8(OP_PING).await?;
        stream.flush().await?;
        let mut response = [0_u8; 5];
        stream.read_exact(&mut response).await?;
        if &response[..4] != MAGIC || response[4] != OP_PING {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid LAN echo response",
            ));
        }
        Result::<(), std::io::Error>::Ok(())
    })
    .await
    .context("LAN echo timed out")??;
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
    let loaded_samples =
        crate::engine::finish_phase(workers, async {}, measure_loaded_latency(server, deadline))
            .await?;
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

async fn open_transfer(server: SocketAddr, opcode: u8, deadline: Instant) -> Result<TcpStream> {
    tokio::time::timeout_at(deadline, async {
        let mut stream = TcpStream::connect(server).await?;
        stream.set_nodelay(true)?;
        stream.write_all(MAGIC).await?;
        stream.write_u8(opcode).await?;
        stream.flush().await?;
        Result::<_, std::io::Error>::Ok(stream)
    })
    .await
    .context("LAN connection/header timed out")?
    .context("failed to open LAN transfer")
}

async fn download_worker(
    server: SocketAddr,
    total: Arc<AtomicU64>,
    deadline: Instant,
) -> Result<()> {
    let mut stream = open_transfer(server, OP_DOWNLOAD, deadline).await?;
    let mut buffer = vec![0_u8; 64 * 1024];
    while Instant::now() < deadline {
        let read = tokio::select! {
            value = stream.read(&mut buffer) => value?,
            _ = sleep_until(deadline) => break,
        };
        anyhow::ensure!(
            read != 0,
            "LAN download ended before its measurement deadline"
        );
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
    for _ in 0..config.streams {
        workers.spawn(upload_worker(server, Arc::clone(&total), deadline));
    }
    let loaded_samples =
        crate::engine::finish_phase(workers, async {}, measure_loaded_latency(server, deadline))
            .await?;
    let bytes = total.load(Ordering::Relaxed);
    anyhow::ensure!(bytes > 0, "LAN server accepted no upload data");
    // Include acknowledgement drain time: buffered data is not instantaneous goodput.
    let seconds = started.elapsed().as_secs_f64();
    Ok((
        ThroughputResult {
            mbps: mbps(bytes, seconds),
            bytes,
            seconds,
        },
        loaded_samples,
    ))
}

async fn upload_worker(server: SocketAddr, total: Arc<AtomicU64>, deadline: Instant) -> Result<()> {
    let mut stream = open_transfer(server, OP_UPLOAD, deadline).await?;
    let buffer = vec![0xa5_u8; IO_CHUNK];
    let mut sent = 0u64;
    while Instant::now() < deadline {
        let write = tokio::select! {
            value = stream.write(&buffer) => value?,
            _ = sleep_until(deadline) => break,
        };
        anyhow::ensure!(write != 0, "LAN upload made no write progress");
        sent += write as u64;
    }
    let acknowledged = timeout(Duration::from_secs(3), async {
        stream.shutdown().await?;
        stream.read_u64().await
    })
    .await
    .context("LAN upload acknowledgement timed out")??;
    anyhow::ensure!(
        acknowledged == sent,
        "LAN upload acknowledgement does not match bytes sent"
    );
    total.fetch_add(acknowledged, Ordering::Relaxed);
    Ok(())
}

async fn measure_loaded_latency(server: SocketAddr, deadline: Instant) -> Vec<f64> {
    let mut samples = Vec::new();
    while Instant::now() < deadline {
        if let Ok(Ok(ms)) = tokio::time::timeout_at(deadline, ping_once(server)).await {
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
}

#[cfg(test)]
mod protocol_tests {
    use super::*;

    #[tokio::test]
    async fn rejects_a_false_echo() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = [0; 5];
            stream.read_exact(&mut header).await.unwrap();
            stream.write_all(b"WRONG").await.unwrap();
        });
        assert!(ping_once(address)
            .await
            .unwrap_err()
            .to_string()
            .contains("invalid LAN echo"));
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_inflated_upload_acknowledgements() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = [0; 5];
            stream.read_exact(&mut header).await.unwrap();
            let mut buffer = [0; 64 * 1024];
            let mut received = 0u64;
            loop {
                let count = stream.read(&mut buffer).await.unwrap();
                if count == 0 {
                    break;
                }
                received += count as u64;
            }
            stream.write_u64(received + 1).await.unwrap();
        });
        let total = Arc::new(AtomicU64::new(0));
        let result = upload_worker(
            address,
            Arc::clone(&total),
            Instant::now() + Duration::from_millis(50),
        )
        .await;
        assert!(result.unwrap_err().to_string().contains("does not match"));
        assert_eq!(total.load(Ordering::Relaxed), 0);
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn server_expires_silent_handshakes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(serve_listener(listener));
        let mut stream = TcpStream::connect(address).await.unwrap();
        let mut byte = [0];
        let count = timeout(Duration::from_secs(4), stream.read(&mut byte))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(count, 0);
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn loaded_probe_never_escapes_phase_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let result = timeout(
            Duration::from_secs(1),
            measure_loaded_latency(address, Instant::now() + Duration::from_millis(30)),
        )
        .await
        .unwrap();
        assert!(result.is_empty());
    }
}
