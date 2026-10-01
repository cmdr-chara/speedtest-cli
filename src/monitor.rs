//! JSONL records for explicit, repeatable connection monitoring.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::TestResult;

pub const SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorRecord {
    pub schema_version: u8,
    pub sequence: u32,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<TestResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl MonitorRecord {
    pub fn success(
        sequence: u32,
        started_at: DateTime<Utc>,
        completed_at: DateTime<Utc>,
        result: TestResult,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            sequence,
            started_at,
            completed_at,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(
        sequence: u32,
        started_at: DateTime<Utc>,
        completed_at: DateTime<Utc>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            sequence,
            started_at,
            completed_at,
            ok: false,
            result: None,
            error: Some(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_record_is_explicit_and_versioned() {
        let record = MonitorRecord::failure(2, Utc::now(), Utc::now(), "fixture failed");
        assert_eq!(record.schema_version, SCHEMA_VERSION);
        assert!(!record.ok);
        assert_eq!(record.error.as_deref(), Some("fixture failed"));
        assert!(record.result.is_none());
    }
}
