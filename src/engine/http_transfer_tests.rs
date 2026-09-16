use super::*;
use std::{task::Poll, time::Duration};

#[tokio::test]
async fn upload_chunks_share_static_storage_and_account_only_polled_bytes() {
    for length in [0, 1, 16 * 1024, 64 * 1024, 64 * 1024 + 7, 8 * 1024 * 1024] {
        let submitted = Arc::new(AtomicU64::new(0));
        let stream = upload_stream(Arc::clone(&submitted), length);
        futures_util::pin_mut!(stream);
        assert_eq!(submitted.load(Ordering::Relaxed), 0);
        let mut observed = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            assert!(!chunk.is_empty());
            assert!(chunk.len() <= UPLOAD_CHUNK.len());
            assert_eq!(chunk.as_ptr(), UPLOAD_CHUNK.as_ptr());
            assert!(chunk.iter().all(|byte| *byte == 0));
            observed += chunk.len();
            assert_eq!(submitted.load(Ordering::Relaxed), observed as u64);
        }
        assert_eq!(observed, length);
        assert!(stream.next().await.is_none());
        assert_eq!(submitted.load(Ordering::Relaxed), length as u64);
    }
}

#[tokio::test]
async fn dropping_an_upload_does_not_count_unpolled_payload() {
    let submitted = Arc::new(AtomicU64::new(0));
    {
        let stream = upload_stream(Arc::clone(&submitted), 8 * 1024 * 1024);
        futures_util::pin_mut!(stream);
        assert_eq!(stream.next().await.unwrap().unwrap().len(), 64 * 1024);
    }
    assert_eq!(submitted.load(Ordering::Relaxed), 64 * 1024);
}

#[tokio::test]
async fn download_counts_complete_streams_without_retaining_chunks() {
    let total = AtomicU64::new(10);
    let stream = futures_util::stream::iter([
        Ok::<_, std::io::Error>(Bytes::from_static(b"abc")),
        Ok(Bytes::new()),
        Ok(Bytes::from_static(b"defgh")),
    ]);
    let result = count_download(stream, &total, Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    assert!(result.completed);
    assert_eq!(result.bytes, 8);
    assert_eq!(total.load(Ordering::Relaxed), 18);
}

#[tokio::test]
async fn download_preserves_partial_goodput_when_the_body_stalls() {
    let total = AtomicU64::new(0);
    let stream = futures_util::stream::once(async {
        Ok::<_, std::io::Error>(Bytes::from_static(b"received"))
    })
    .chain(futures_util::stream::pending());
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        count_download(stream, &total, Instant::now() + Duration::from_millis(100)),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!result.completed);
    assert_eq!(result.bytes, 8);
    assert_eq!(total.load(Ordering::Relaxed), 8);
}

#[tokio::test]
async fn download_propagates_body_errors_before_the_cutoff() {
    let total = AtomicU64::new(0);
    let stream = futures_util::stream::iter([
        Ok(Bytes::from_static(b"abc")),
        Err(std::io::Error::other("fixture truncated body")),
    ]);
    let error = count_download(stream, &total, Instant::now() + Duration::from_secs(1))
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("fixture truncated body"));
    assert_eq!(total.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn expired_download_does_not_poll_an_already_ready_body() {
    let total = AtomicU64::new(0);
    let mut polled = false;
    let stream = futures_util::stream::poll_fn(|_| {
        polled = true;
        Poll::Ready(Some(Ok::<_, std::io::Error>(Bytes::from_static(b"late"))))
    });
    let result = count_download(stream, &total, Instant::now()).await.unwrap();
    assert!(!polled);
    assert!(!result.completed);
    assert_eq!(result.bytes, 0);
    assert_eq!(total.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn chunks_finishing_a_long_poll_after_the_cutoff_are_excluded() {
    let total = AtomicU64::new(0);
    let deadline = Instant::now() + Duration::from_millis(100);
    let mut polls = 0;
    let stream = futures_util::stream::poll_fn(|_| {
        polls += 1;
        if polls > 1 {
            std::thread::sleep(
                (deadline + Duration::from_millis(10)).saturating_duration_since(Instant::now()),
            );
        }
        Poll::Ready(Some(Ok::<_, std::io::Error>(Bytes::from_static(b"data"))))
    });
    let result = count_download(stream, &total, deadline).await.unwrap();
    assert_eq!(polls, 2, "exercise the post-poll cutoff, not just the timer");
    assert!(!result.completed);
    assert_eq!(result.bytes, 4);
    assert_eq!(total.load(Ordering::Relaxed), 4);
}

#[tokio::test]
async fn an_expired_deadline_does_not_poll_work() {
    let mut polled = false;
    let result = before_deadline(Instant::now(), async { polled = true }).await;
    assert!(result.is_none());
    assert!(!polled);
}

#[tokio::test]
async fn completed_work_is_returned_before_the_deadline() {
    assert_eq!(
        before_deadline(Instant::now() + Duration::from_secs(1), async { 42 }).await,
        Some(42)
    );
}

#[tokio::test]
async fn deadline_cancels_a_stalled_future() {
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        before_deadline(
            Instant::now() + Duration::from_millis(50),
            std::future::pending::<()>(),
        ),
    )
    .await
    .unwrap();
    assert!(result.is_none());
}
