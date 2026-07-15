//! One absolute work budget shared across a complete Lambda invocation.
//!
//! Phases cannot each restart the timeout. Multipart cleanup intentionally runs
//! outside this budget after an expired operation has surrendered ownership.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::time::{Instant, timeout_at};

use crate::error::DeploymentError;

type DeadlineFuture<'future, T> =
    Pin<Box<dyn Future<Output = Result<T, DeploymentError>> + Send + 'future>>;

/// One absolute work deadline shared by every phase of an invocation.
///
/// Publication catches expiry while it still owns any multipart upload, so it
/// can attempt the mandatory abort before the handler returns.
#[derive(Clone, Copy)]
pub(crate) struct InvocationDeadline {
    expires: Instant,
    budget: Duration,
}

impl InvocationDeadline {
    pub(crate) fn after(budget: Duration) -> Self {
        let expires = Instant::now()
            .checked_add(budget)
            .expect("the fixed Lambda work deadline fits Tokio's clock domain");
        Self { expires, budget }
    }

    /// Boxes a phase so the long-lived handler does not retain every large
    /// future state machine in one stack frame.
    pub(crate) fn run<'future, T, E>(
        self,
        future: impl Future<Output = Result<T, E>> + Send + 'future,
    ) -> DeadlineFuture<'future, T>
    where
        T: Send + 'future,
        E: Send + 'future,
        DeploymentError: From<E>,
    {
        Box::pin(async move {
            timeout_at(self.expires, future)
                .await
                .map_err(|_| DeploymentError::invocation_timeout(self.budget))?
                .map_err(DeploymentError::from)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::InvocationDeadline;
    use crate::error::DeploymentError;

    #[tokio::test]
    async fn bounds_the_complete_invocation_pipeline() {
        let deadline = InvocationDeadline::after(Duration::ZERO);
        let result = deadline
            .run(std::future::pending::<Result<(), DeploymentError>>())
            .await;

        assert!(matches!(
            result,
            Err(DeploymentError::InvocationTimeout { .. })
        ));
    }
}
