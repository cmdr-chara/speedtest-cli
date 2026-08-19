use chrono::{TimeZone, Utc};
use speedtest_cli::model::{LatencyResult, ServerInfo, TestResult, ThroughputResult};

#[test]
fn canonical_result_round_trips_through_json() {
    let result = TestResult {
        timestamp: Utc.with_ymd_and_hms(2026, 8, 19, 17, 0, 0).unwrap(),
        backend: "cloudflare".into(),
        server: ServerInfo {
            host: "speed.cloudflare.com".into(),
            name: "Cloudflare Edge".into(),
        },
        latency: LatencyResult {
            idle_ms: 8.2,
            jitter_ms: 1.1,
            download_loaded_ms: Some(19.4),
            upload_loaded_ms: Some(17.8),
            packet_loss_percent: None,
        },
        download: ThroughputResult {
            mbps: 842.6,
            bytes: 842_600_000,
            seconds: 8.0,
        },
        upload: ThroughputResult {
            mbps: 293.4,
            bytes: 293_400_000,
            seconds: 8.0,
        },
    };

    let json = serde_json::to_string(&result).unwrap();
    let decoded: TestResult = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded.backend, "cloudflare");
    assert_eq!(decoded.server.host, "speed.cloudflare.com");
    assert!((decoded.download.mbps - 842.6).abs() < f64::EPSILON);
}
