//! Explicit HTTP/2 and HTTP/3 capability probes.

use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use crate::engine::AddressFamily;

const PROBE_URL: &str = "https://cloudflare.com/cdn-cgi/trace";

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportProtocol {
    Http2,
    Http3,
}

impl TransportProtocol {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Http2 => "http/2",
            Self::Http3 => "http/3",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportProbe {
    pub timestamp: DateTime<Utc>,
    pub protocol: TransportProtocol,
    pub ok: bool,
    pub elapsed_ms: Option<f64>,
    pub negotiated_version: Option<String>,
    pub error: Option<String>,
}

pub async fn probe(family: AddressFamily, protocol: TransportProtocol) -> TransportProbe {
    let started = Instant::now();
    let builder = Client::builder()
        .user_agent(concat!("speedtest-cli/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(8))
        .local_address(family.local_address(None));
    let client = match protocol {
        TransportProtocol::Http2 => builder.http2_prior_knowledge().build(),
        TransportProtocol::Http3 => builder.http3_prior_knowledge().build(),
    };
    let result = match client {
        Ok(client) => match client.get(PROBE_URL).send().await {
            Ok(response) => {
                let version = format!("{:?}", response.version());
                match response.bytes().await {
                    Ok(_) => Ok(version),
                    Err(error) => Err(format!("response body failed: {error}")),
                }
            }
            Err(error) => Err(format!("{} request failed: {error}", protocol.label())),
        },
        Err(error) => Err(format!("{} client setup failed: {error}", protocol.label())),
    };
    match result {
        Ok(version) => TransportProbe {
            timestamp: Utc::now(),
            protocol,
            ok: true,
            elapsed_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
            negotiated_version: Some(version),
            error: None,
        },
        Err(error) => TransportProbe {
            timestamp: Utc::now(),
            protocol,
            ok: false,
            elapsed_ms: None,
            negotiated_version: None,
            error: Some(error),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_labels_are_stable_machine_values() {
        assert_eq!(TransportProtocol::Http2.label(), "http/2");
        assert_eq!(TransportProtocol::Http3.label(), "http/3");
    }
}
