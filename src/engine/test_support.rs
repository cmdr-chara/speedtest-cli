//! Minimal bounded local HTTP peer for failure-path tests.
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

pub async fn upload_peer(replies: Vec<(u16, Duration)>) -> (String, JoinHandle<Vec<usize>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut lengths = Vec::new();
        for (status, delay) in replies {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            loop {
                header.push(stream.read_u8().await.unwrap());
                assert!(header.len() < 8192);
                if header.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let header = String::from_utf8(header).unwrap().to_ascii_lowercase();
            let size: usize = header
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            assert!(size <= 25_000_000);
            let mut left = size;
            let mut buffer = [0; 8192];
            while left > 0 {
                let take = left.min(buffer.len());
                let count = stream.read(&mut buffer[..take]).await.unwrap();
                if count == 0 {
                    break;
                }
                left -= count;
            }
            lengths.push(size - left);
            tokio::time::sleep(delay).await;
            let response = format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
        lengths
    });
    (url, task)
}

/// A bounded, concurrent peer for complete phase/event tests. It supports the
/// standard LibreSpeed paths and keeps all transfer data on loopback.
pub async fn measurement_peer() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept(), if connections.len() < 16 => {
                    let (stream, _) = accepted.unwrap();
                    connections.spawn(reply_to_measurement(stream));
                }
                Some(result) = connections.join_next(), if !connections.is_empty() => {
                    result.unwrap();
                }
            }
        }
    });
    (url, task)
}

async fn reply_to_measurement(mut stream: tokio::net::TcpStream) {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        match stream.read_u8().await {
            Ok(byte) => header.push(byte),
            Err(_) => return,
        }
        assert!(header.len() < 8192);
    }
    let header = String::from_utf8(header).unwrap().to_ascii_lowercase();
    let upload = header.starts_with("post ");
    let download = header.starts_with("get /garbage.php?");
    if upload {
        let mut left: usize = header
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(left <= 8 * 1024 * 1024);
        let mut buffer = [0; 8192];
        while left > 0 {
            let take = left.min(buffer.len());
            match stream.read(&mut buffer[..take]).await {
                Ok(0) | Err(_) => return,
                Ok(count) => left -= count,
            }
        }
    }
    tokio::time::sleep(Duration::from_millis(if upload { 40 } else { 5 })).await;
    let body_len = if download { 64 * 1024 } else { 0 };
    let response =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {body_len}\r\nConnection: close\r\n\r\n");
    if stream.write_all(response.as_bytes()).await.is_ok() && download {
        let _ = stream.write_all(&vec![0x5a; body_len]).await;
    }
}
