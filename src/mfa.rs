//! One cooperative, monotonic deadline for MFA setup, checks and retry sleeps.

use std::{future::Future, ops::AsyncFnMut, time::Duration};

use anyhow::{Context, Result};
use bourso_api::client::error::ClientError;

pub(crate) const VALIDATION_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Wrap the complete operation, including challenge setup, in one budget.
/// Expiry drops the operation and its in-flight request. Tokio cancellation is
/// cooperative: synchronous blocking work cannot be preempted by this deadline.
pub(crate) async fn within_deadline<T>(
    budget: Duration,
    operation: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::time::timeout(budget, operation)
        .await
        .with_context(|| {
            format!(
                "MFA validation timed out after {} seconds",
                budget.as_secs_f64()
            )
        })?
}

/// Poll immediately, then sleep between pending responses. The caller must put
/// setup and this loop inside `within_deadline`; this loop never resets a budget.
/// AsyncFnMut permits borrowing the same mutable client across each request.
pub(crate) async fn wait_until<F>(interval: Duration, mut check: F) -> Result<()>
where
    F: AsyncFnMut() -> Result<bool>,
{
    loop {
        if check().await? {
            return Ok(());
        }
        tokio::time::sleep(interval).await;
    }
}

/// A newly displayed QR is still pending, not a new validation attempt/budget.
/// Keep the response mapping shared with fixtures as well as the live caller.
pub(crate) fn validation_status(
    response: Result<bool>,
    mut display_qr: impl FnMut(&str) -> Result<()>,
) -> Result<bool> {
    match response {
        Ok(validated) => Ok(validated),
        Err(error) => match error.downcast_ref() {
            Some(ClientError::QRCodeRequired(code)) => {
                display_qr(code)?;
                Ok(false)
            }
            _ => Err(error),
        },
    }
}
