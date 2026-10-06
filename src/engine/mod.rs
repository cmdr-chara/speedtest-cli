#[path = "cloudflare_adaptive.rs"]
pub mod cloudflare;
pub(crate) mod http;
pub mod internet;
pub mod librespeed;

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    time::Duration,
};

use crate::model::{TestPhase, TestResult};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub streams: usize,
    pub phase_duration: Duration,
    pub family: AddressFamily,
    pub source_ip: Option<IpAddr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AddressFamily {
    #[default]
    Any,
    Ipv4,
    Ipv6,
}

impl AddressFamily {
    /// An unspecified local address asks the socket stack to use only the
    /// requested address family without guessing a machine-specific address.
    pub const fn local_address(self, source_ip: Option<IpAddr>) -> Option<IpAddr> {
        if let Some(source_ip) = source_ip {
            return Some(source_ip);
        }
        match self {
            Self::Any => None,
            Self::Ipv4 => Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            Self::Ipv6 => Some(IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Ipv4 => "ipv4",
            Self::Ipv6 => "ipv6",
        }
    }
}

#[derive(Debug, Clone)]
pub enum EngineEvent {
    PhaseChanged(TestPhase),
    IdleLatency { ping_ms: f64, jitter_ms: f64 },
    ThroughputSample { phase: TestPhase, mbps: f64 },
    LoadedLatency { phase: TestPhase, ms: f64 },
    Complete(TestResult),
    Error(String),
}

impl EngineConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=16).contains(&self.streams),
            "streams must be between 1 and 16"
        );
        anyhow::ensure!(
            !self.phase_duration.is_zero() && self.phase_duration <= Duration::from_secs(30),
            "phase duration must be positive and at most 30 seconds"
        );
        if let Some(source_ip) = self.source_ip {
            anyhow::ensure!(
                matches!(
                    (self.family, source_ip),
                    (AddressFamily::Any, IpAddr::V4(_))
                        | (AddressFamily::Any, IpAddr::V6(_))
                        | (AddressFamily::Ipv4, IpAddr::V4(_))
                        | (AddressFamily::Ipv6, IpAddr::V6(_))
                ),
                "source IP address family does not match --family"
            );
        }
        Ok(())
    }
}

/// Keep latency probes, sampling, and workers in one cancellation scope.
pub(crate) async fn finish_phase(
    mut workers: tokio::task::JoinSet<anyhow::Result<()>>,
    samples: impl std::future::Future<Output = ()>,
    loaded: impl std::future::Future<Output = Vec<f64>>,
) -> anyhow::Result<Vec<f64>> {
    use anyhow::Context;
    let result = {
        let collect = async {
            while let Some(result) = workers.join_next().await {
                result.context("transfer worker failed")??;
            }
            Ok::<_, anyhow::Error>(())
        };
        tokio::try_join!(
            collect,
            async {
                samples.await;
                Ok(())
            },
            async { Ok(loaded.await) },
        )
    };
    if result.is_err() {
        // Abort AND join siblings before reporting failure. Dropping a JoinSet
        // only requests cancellation and can leave worker cleanup pending.
        workers.shutdown().await;
    }
    result.map(|(_, _, latency)| latency)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unbounded_library_configuration() {
        for streams in [0, 17, usize::MAX] {
            assert!(EngineConfig {
                streams,
                phase_duration: Duration::from_secs(1),
                family: AddressFamily::Any,
                source_ip: None,
            }
            .validate()
            .is_err());
        }
        assert!(EngineConfig {
            streams: 1,
            phase_duration: Duration::ZERO,
            family: AddressFamily::Any,
            source_ip: None,
        }
        .validate()
        .is_err());

        assert!(EngineConfig {
            streams: 1,
            phase_duration: Duration::from_secs(1),
            family: AddressFamily::Ipv6,
            source_ip: Some("192.0.2.10".parse().unwrap()),
        }
        .validate()
        .is_err());
        assert!(EngineConfig {
            streams: 1,
            phase_duration: Duration::from_secs(1),
            family: AddressFamily::Ipv4,
            source_ip: Some("192.0.2.10".parse().unwrap()),
        }
        .validate()
        .is_ok());
    }

    #[tokio::test]
    async fn worker_failure_does_not_wait_for_sampler_or_loaded_probe() {
        let mut workers = tokio::task::JoinSet::new();
        workers.spawn(async { anyhow::bail!("fixture failure") });
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            finish_phase(workers, std::future::pending(), std::future::pending()),
        )
        .await;
        assert!(result.unwrap().is_err());
    }

    struct DropSignal(std::sync::Arc<std::sync::atomic::AtomicBool>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn worker_failure_joins_cancelled_siblings_before_returning() {
        for panic in [false, true] {
            let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let signal = DropSignal(std::sync::Arc::clone(&dropped));
            let (started, ready) = tokio::sync::oneshot::channel();
            let mut workers = tokio::task::JoinSet::new();
            workers.spawn(async move {
                let _signal = signal;
                started.send(()).unwrap();
                std::future::pending::<anyhow::Result<()>>().await
            });
            workers.spawn(async move {
                ready.await.unwrap();
                assert!(!panic, "fixture worker panic");
                anyhow::bail!("fixture worker error")
            });
            let error = tokio::time::timeout(
                Duration::from_secs(2),
                finish_phase(workers, std::future::pending(), std::future::pending()),
            )
            .await
            .unwrap()
            .unwrap_err();
            assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
            assert!(error.to_string().contains(if panic {
                "transfer worker failed"
            } else {
                "fixture worker error"
            }));
        }
    }

    #[tokio::test]
    async fn successful_phase_preserves_loaded_samples() {
        let mut workers = tokio::task::JoinSet::new();
        workers.spawn(async { Ok(()) });
        let samples = finish_phase(workers, async {}, async { vec![1.0, 2.0] })
            .await
            .unwrap();
        assert_eq!(samples, vec![1.0, 2.0]);
    }
}

#[cfg(test)]
mod test_support;
