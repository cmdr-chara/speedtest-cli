//! HTTP contracts shared by measurement engines, not general-purpose browsing.
use bytes::Bytes;
use reqwest::Body;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use reqwest::{Response, Url};
use tokio::time::{sleep_until, Instant};

pub fn success(response: Response) -> Result<Response> {
    if !response.status().is_success() {
        bail!("measurement endpoint returned HTTP {}; redirects are not followed; use the final server URL", response.status());
    }
    Ok(response)
}

/// Drain small control responses without retaining or accepting arbitrary bodies.
pub async fn drain(response: Response, limit: usize) -> Result<()> {
    let response = success(response)?;
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        bail!("measurement control response exceeds {limit} bytes");
    }
    let mut stream = response.bytes_stream();
    let mut received = 0usize;
    while let Some(chunk) = stream.next().await {
        received = received.saturating_add(chunk?.len());
        if received > limit {
            bail!("measurement control response exceeds {limit} bytes");
        }
    }
    Ok(())
}

/// Commit only uploads whose complete successful response was observed before the
/// cutoff. An async response body can finish a long poll after its deadline, even
/// when it is raced against a timer, so check the clock after it returns as well.
pub(crate) async fn complete_upload(
    response_body: impl std::future::Future<Output = Result<()>>,
    submitted: &AtomicU64,
    payload_len: usize,
    total: &AtomicU64,
    deadline: Instant,
) -> Result<bool> {
    if Instant::now() >= deadline {
        return Ok(false);
    }
    let result = tokio::select! {
        biased;
        _ = sleep_until(deadline) => return Ok(false),
        result = response_body => result,
    };
    if Instant::now() >= deadline {
        return Ok(false);
    }
    result?;
    anyhow::ensure!(
        submitted.load(Ordering::Relaxed) == payload_len as u64,
        "upload endpoint responded before the complete request body was submitted"
    );
    total.fetch_add(payload_len as u64, Ordering::Relaxed);
    Ok(true)
}

pub fn base_url(value: &str) -> Result<Url> {
    if value.chars().any(|c| c.is_control() || c.is_whitespace()) {
        bail!("server URL must not contain whitespace or control characters");
    }
    let mut url = Url::parse(value).context("invalid server URL; expected http:// or https://")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("server URL must use HTTP or HTTPS with a host");
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("server URL must not contain credentials, query parameters, or a fragment; these could be persisted in history");
    }
    let path = format!("{}/", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url)
}

pub(crate) fn upload_body(total: Arc<AtomicU64>, payload_len: usize) -> Body {
    let chunk = Bytes::from(vec![0_u8; 64 * 1024]);
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_secret_bearing_and_non_http_urls_without_echoing_them() {
        for value in [
            "https://user:secret@example.test/",
            "https://example.test/?token=secret",
            "https://example.test/#secret",
            "file:///tmp/x",
            "ftp://example.test",
            "https://example.test/\n",
        ] {
            let error = base_url(value).unwrap_err().to_string();
            assert!(!error.contains("secret"));
        }
        assert_eq!(
            base_url("http://127.0.0.1:9876/backend").unwrap().as_str(),
            "http://127.0.0.1:9876/backend/"
        );
        assert_eq!(
            base_url("https://[::1]/").unwrap().as_str(),
            "https://[::1]/"
        );
    }
}

#[cfg(test)]
mod response_tests {
    use super::*;
    use std::time::Duration;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[tokio::test]
    async fn rejects_redirects_and_oversized_control_bodies() {
        for response in [
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 9999999\r\n\r\n",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nabcd\r\n0\r\n\r\n",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let peer = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    header.push(stream.read_u8().await.unwrap());
                    assert!(header.len() < 8192);
                }
                stream.write_all(response.as_bytes()).await.unwrap();
            });
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap();
            let response = client
                .get(format!("http://{address}/"))
                .send()
                .await
                .unwrap();
            assert!(drain(response, 3).await.is_err());
            peer.await.unwrap();
        }
    }

    #[tokio::test]
    async fn upload_finishing_in_one_poll_after_cutoff_is_not_counted() {
        let submitted = AtomicU64::new(64 * 1024);
        let total = AtomicU64::new(0);
        let deadline = Instant::now() + Duration::from_millis(100);
        let mut polled = false;
        let response_body = async {
            polled = true;
            // Model response processing that does not yield back to the timer.
            // The completion guard used by both HTTP engines must still exclude it.
            std::thread::sleep(
                (deadline + Duration::from_millis(10)).saturating_duration_since(Instant::now()),
            );
            Ok(())
        };
        assert!(
            !complete_upload(response_body, &submitted, 64 * 1024, &total, deadline)
                .await
                .unwrap()
        );
        assert!(
            polled,
            "the response must cross the cutoff while being polled"
        );
        assert_eq!(total.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn expired_upload_does_not_poll_an_already_ready_response() {
        let total = AtomicU64::new(0);
        let submitted = AtomicU64::new(64 * 1024);
        let mut polled = false;
        let response_body = async {
            polled = true;
            Ok(())
        };
        assert!(
            !complete_upload(response_body, &submitted, 64 * 1024, &total, Instant::now(),)
                .await
                .unwrap()
        );
        assert!(!polled);
        assert_eq!(total.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn only_complete_successful_upload_responses_commit_payload_bytes() {
        let total = AtomicU64::new(0);
        let submitted = AtomicU64::new(64 * 1024);
        let deadline = Instant::now() + Duration::from_secs(1);
        assert!(
            complete_upload(async { Ok(()) }, &submitted, 64 * 1024, &total, deadline)
                .await
                .unwrap()
        );
        assert_eq!(total.load(Ordering::Relaxed), 64 * 1024);

        assert!(complete_upload(
            async { anyhow::bail!("truncated response") },
            &submitted,
            64 * 1024,
            &total,
            deadline,
        )
        .await
        .is_err());
        submitted.store(32 * 1024, Ordering::Relaxed);
        assert!(
            complete_upload(async { Ok(()) }, &submitted, 64 * 1024, &total, deadline)
                .await
                .is_err()
        );
        assert_eq!(total.load(Ordering::Relaxed), 64 * 1024);
    }
}
