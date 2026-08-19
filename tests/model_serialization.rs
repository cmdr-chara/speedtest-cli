use chrono::{TimeZone, Utc};
use speedtest_cli::model::{
    BufferbloatAssessment, DiagnosticFinding, FindingSeverity, LatencyAnalysis,
    LatencyDistribution, LatencyResult, NetworkAnalysis, NetworkQuality, QualityConfidence,
    QualityGrade, ServerInfo, TestResult, ThroughputResult, WorkloadGrades,
};

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
        analysis: Some(Box::new(NetworkAnalysis {
            latency: LatencyAnalysis {
                idle: LatencyDistribution {
                    samples: 24,
                    min_ms: 7.9,
                    median_ms: 8.2,
                    p95_ms: 10.1,
                    p99_ms: 11.0,
                    max_ms: 11.2,
                },
                jitter: Some(LatencyDistribution {
                    samples: 23,
                    min_ms: 0.1,
                    median_ms: 0.8,
                    p95_ms: 2.0,
                    p99_ms: 2.4,
                    max_ms: 2.5,
                }),
                download_loaded: None,
                upload_loaded: None,
            },
            quality: NetworkQuality {
                score: 94,
                grade: QualityGrade::A,
                confidence: QualityConfidence::Moderate,
                bufferbloat: BufferbloatAssessment {
                    download_increase_ms: Some(11.2),
                    upload_increase_ms: Some(9.6),
                    worst_increase_ms: Some(11.2),
                    grade: Some(QualityGrade::A),
                },
                workloads: WorkloadGrades {
                    gaming: QualityGrade::A,
                    video_calls: QualityGrade::A,
                    streaming: QualityGrade::APlus,
                    cloud_gaming: QualityGrade::A,
                },
                findings: vec![DiagnosticFinding {
                    severity: FindingSeverity::Info,
                    title: "Connection looks healthy".into(),
                    evidence: "Tail latency remains controlled.".into(),
                    recommendation: None,
                }],
            },
        })),
    };

    let json = serde_json::to_string(&result).unwrap();
    let decoded: TestResult = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded.backend, "cloudflare");
    assert_eq!(decoded.server.host, "speed.cloudflare.com");
    assert!((decoded.download.mbps - 842.6).abs() < f64::EPSILON);
    assert_eq!(decoded.analysis.unwrap().quality.score, 94);
}

#[test]
fn legacy_result_without_analysis_still_deserializes() {
    let json = r#"{
        "timestamp":"2026-08-19T17:00:00Z",
        "backend":"cloudflare",
        "server":{"host":"speed.cloudflare.com","name":"Cloudflare Edge"},
        "latency":{"idle_ms":8.2,"jitter_ms":1.1,"download_loaded_ms":19.4,"upload_loaded_ms":17.8,"packet_loss_percent":null},
        "download":{"mbps":842.6,"bytes":842600000,"seconds":8.0},
        "upload":{"mbps":293.4,"bytes":293400000,"seconds":8.0}
    }"#;

    let decoded: TestResult = serde_json::from_str(json).unwrap();
    assert!(decoded.analysis.is_none());
}
