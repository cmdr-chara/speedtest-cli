use super::*;
use std::sync::atomic::AtomicBool;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn candidate(name: &str) -> ResolvedServer {
    let mut server = resolve_custom_server("http://127.0.0.1:1/").unwrap();
    server.name = name.to_string();
    server
}

#[tokio::test]
async fn selection_ranks_medians_instead_of_completion_order() {
    let mut workers = JoinSet::new();
    workers.spawn(async { Ok((candidate("slow"), 200.0, 0)) });
    workers.spawn(async { Ok((candidate("fast"), 5.0, 1)) });
    workers.spawn(async { Ok((candidate("middle"), 30.0, 2)) });
    let selected = select_candidates(workers, Instant::now() + Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(selected.name, "fast");
}

#[tokio::test]
async fn equal_medians_keep_registry_order() {
    let mut workers = JoinSet::new();
    workers.spawn(async { Ok((candidate("second"), 5.0, 1)) });
    workers.spawn(async { Ok((candidate("first"), 5.0, 0)) });
    let selected = select_candidates(workers, Instant::now() + Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(selected.name, "first");
}

#[tokio::test]
async fn failed_and_non_finite_candidates_cannot_win_selection() {
    let mut workers = JoinSet::new();
    workers.spawn(async { anyhow::bail!("fixture unavailable") });
    workers.spawn(async { Ok((candidate("nan"), f64::NAN, 0)) });
    workers.spawn(async { Ok((candidate("infinite"), f64::INFINITY, 1)) });
    workers.spawn(async { Ok((candidate("healthy"), 10.0, 2)) });
    let selected = select_candidates(workers, Instant::now() + Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(selected.name, "healthy");
}

#[tokio::test]
async fn an_empty_or_failed_registry_returns_the_existing_error() {
    for failed in [false, true] {
        let mut workers = JoinSet::new();
        if failed {
            workers.spawn(async { anyhow::bail!("fixture unavailable") });
        }
        let error = select_candidates(workers, Instant::now() + Duration::from_secs(2))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "no built-in LibreSpeed server was reachable"
        );
    }
}

struct DropSignal(Arc<AtomicBool>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn selection_timeout_joins_stalled_candidates_and_keeps_a_healthy_result() {
    let dropped = Arc::new(AtomicBool::new(false));
    let signal = DropSignal(Arc::clone(&dropped));
    let (started, ready) = tokio::sync::oneshot::channel();
    let mut workers = JoinSet::new();
    workers.spawn(async move {
        let _signal = signal;
        started.send(()).unwrap();
        std::future::pending().await
    });
    workers.spawn(async { Ok((candidate("healthy"), 5.0, 0)) });
    ready.await.unwrap();
    let selected = tokio::time::timeout(
        Duration::from_secs(2),
        select_candidates(workers, Instant::now() + Duration::from_millis(100)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(selected.name, "healthy");
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancelling_selection_does_not_detach_probe_workers() {
    let dropped = Arc::new(AtomicBool::new(false));
    let signal = DropSignal(Arc::clone(&dropped));
    let (started, ready) = tokio::sync::oneshot::channel();
    let mut workers = JoinSet::new();
    workers.spawn(async move {
        let _signal = signal;
        started.send(()).unwrap();
        std::future::pending().await
    });
    let task = tokio::spawn(select_candidates(
        workers,
        Instant::now() + Duration::from_secs(10),
    ));
    ready.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(2), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn loopback_selection_cancels_a_stalled_response_body() {
    let (url, healthy_peer) = crate::engine::test_support::measurement_peer().await;
    let healthy = resolve_custom_server(&url).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stalled =
        resolve_custom_server(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let stalled_peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(stream.read_u8().await.unwrap());
            assert!(header.len() < 8192);
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n")
            .await
            .unwrap();
        // Deliberately never send the body. Selection must close this connection.
        let mut byte = [0];
        assert_eq!(stream.read(&mut byte).await.unwrap(), 0);
    });
    let client = Client::builder().no_proxy().build().unwrap();
    let selected = tokio::time::timeout(
        Duration::from_secs(3),
        select_server(
            &client,
            vec![stalled, healthy.clone()],
            Instant::now() + Duration::from_millis(750),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(selected.base, healthy.base);
    tokio::time::timeout(Duration::from_secs(2), stalled_peer)
        .await
        .unwrap()
        .unwrap();
    healthy_peer.abort();
    let _ = healthy_peer.await;
}

#[tokio::test]
async fn an_expired_sampler_emits_no_throughput() {
    let engine = LibreSpeedEngine {
        client: Client::builder().no_proxy().build().unwrap(),
        config: EngineConfig {
            streams: 1,
            phase_duration: Duration::from_secs(1),
        },
        server: None,
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    engine
        .sample_transfer(
            TestPhase::Download,
            Arc::new(AtomicU64::new(1_000_000)),
            Instant::now(),
            &tx,
        )
        .await;
    assert!(rx.try_recv().is_err());
}
