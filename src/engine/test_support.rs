//! Minimal bounded local HTTP peer for failure-path tests.
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
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
