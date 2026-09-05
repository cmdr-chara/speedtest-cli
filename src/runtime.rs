//! Process-level outcomes shared by the terminal and CLI boundary.
use std::{fmt, future::Future, io, time::Duration};

use anyhow::Result;

#[derive(Debug)]
pub enum Outcome {
    Cancelled,
    TimedOut,
    ThresholdFailed,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "cancelled; no incomplete result was saved",
            Self::TimedOut => {
                "measurement deadline exceeded; check connectivity or increase --timeout"
            }
            Self::ThresholdFailed => "one or more thresholds failed",
        })
    }
}
impl std::error::Error for Outcome {}

pub fn exit_code(error: &anyhow::Error) -> u8 {
    if error.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
    }) {
        return 0;
    }
    match error.downcast_ref::<Outcome>() {
        Some(Outcome::Cancelled) => 130,
        Some(Outcome::TimedOut) => 124,
        Some(Outcome::ThresholdFailed) => 3,
        None => 1,
    }
}

pub async fn deadline<T>(duration: Duration, future: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(duration, future)
        .await
        .map_err(|_| Outcome::TimedOut)?
}

/// Dropping the owned future cancels its I/O. Callers must not detach child tasks.
pub async fn interruptible<T>(future: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::select! {
        result = future => result,
        signal = tokio::signal::ctrl_c() => {
            signal?;
            Err(Outcome::Cancelled.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_exit_codes_include_context_wrapped_broken_pipes() {
        assert_eq!(
            exit_code(
                &anyhow::Error::new(io::Error::from(io::ErrorKind::BrokenPipe)).context("stdout")
            ),
            0
        );
        assert_eq!(exit_code(&Outcome::Cancelled.into()), 130);
        assert_eq!(exit_code(&Outcome::TimedOut.into()), 124);
        assert_eq!(exit_code(&Outcome::ThresholdFailed.into()), 3);
    }

    #[tokio::test]
    async fn deadline_drops_the_owned_future() {
        let result: Result<()> = deadline(Duration::from_millis(1), std::future::pending()).await;
        assert_eq!(exit_code(&result.unwrap_err()), 124);
    }
}
