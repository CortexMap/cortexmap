use crate::FetchError;
use backon::{ExponentialBuilder, Retryable};
use cortexmap_core::blueprint::connections::BackoffStrategy;
use cortexmap_infra::InfraError;
use std::future::Future;
use std::time::Duration;

/// Default number of request-level retries for individual HTTP/S3 calls.
/// This is independent of task-level retries managed by the worker queue.
const DEFAULT_REQUEST_RETRIES: usize = 3;

/// Default maximum delay for request-level retries (30 seconds).
const DEFAULT_REQUEST_MAX_DELAY_SECS: u64 = 30;

/// Default minimum delay for request-level retries (1 second).
const DEFAULT_REQUEST_MIN_DELAY_SECS: u64 = 1;

// ========================== Retryable predicates ==========================

/// Determines whether an `InfraError` is transient and should be retried.
///
/// Retryable:
/// - HTTP timeouts, connection errors, and server errors (429, 5xx)
/// - S3 SDK errors (transient infrastructure failures)
/// - Connection pool exhaustion
///
/// Not retryable:
/// - Database schema/logic errors
/// - Missing environment variables
pub fn is_infra_retryable(err: &InfraError) -> bool {
    match err {
        InfraError::HttpError(e) => {
            // Retry on timeouts and connection failures
            if e.is_timeout() || e.is_connect() || e.is_request() {
                return true;
            }
            // Retry on server errors (5xx) and rate limiting (429)
            if let Some(status) = e.status() {
                return status.as_u16() == 429
                    || status.is_server_error();
            }
            false
        }
        // S3 operations can fail transiently
        InfraError::PutObjectError(_) | InfraError::GetObjectError(_) | InfraError::S3Error(_) => true,
        // Pool exhaustion is transient
        InfraError::R2D2PoolError(_) => true,
        // Database errors and env var errors are not retryable
        InfraError::Database(_) | InfraError::EnvVarNotFound(_) | InfraError::Join(_) => false,
    }
}

/// Determines whether a `FetchError` is transient and should be retried.
///
/// Delegates to `is_infra_retryable` for infrastructure errors.
/// NotFound and InvalidPdfSource are permanent and not retried.
pub fn is_fetch_retryable(err: &FetchError) -> bool {
    match err {
        FetchError::InfraError(e) => is_infra_retryable(e),
        FetchError::ReqwestError(e) => {
            e.is_timeout() || e.is_connect() || e.is_request()
                || e.status().is_some_and(|s| s.as_u16() == 429 || s.is_server_error())
        }
        // Deserialization errors are not transient
        FetchError::SerdeError(_) => false,
        // Task join errors are not retryable
        FetchError::JoinError(_) => false,
        // These are permanent: the resource doesn't exist or is invalid
        FetchError::InvalidPdfSource(_) | FetchError::NotFound(_) => false,
    }
}

// ========================== Request-level retry ==========================

/// Execute an async operation with request-level retry using exponential backoff.
///
/// Uses sensible defaults for request-level retries:
/// - 3 retries
/// - 1s initial delay, exponential growth, 30s max delay
/// - Jitter enabled to prevent thundering herd
///
/// Only retries when the `when` predicate returns true for the error.
pub async fn with_request_retry<T, E, Fut, FutureFn, WhenFn>(
    operation: FutureFn,
    when: WhenFn,
    operation_name: &str,
) -> Result<T, E>
where
    Fut: Future<Output = Result<T, E>>,
    FutureFn: FnMut() -> Fut,
    WhenFn: FnMut(&E) -> bool,
    E: std::fmt::Display,
{
    let name = operation_name.to_string();
    operation
        .retry(
            ExponentialBuilder::default()
                .with_min_delay(Duration::from_secs(DEFAULT_REQUEST_MIN_DELAY_SECS))
                .with_max_delay(Duration::from_secs(DEFAULT_REQUEST_MAX_DELAY_SECS))
                .with_max_times(DEFAULT_REQUEST_RETRIES)
                .with_jitter(),
        )
        .when(when)
        .notify(move |err: &E, dur: Duration| {
            tracing::warn!(
                "Retrying {} after {:?} due to: {}",
                name,
                dur,
                err
            );
        })
        .await
}

// ========================== Task-level backoff delay ==========================

/// Compute the backoff delay for task-level retries based on the configured strategy.
///
/// This is used in the worker loop to determine how long to sleep after a task
/// fails before the worker picks up the next task.
///
/// # Arguments
/// - `strategy`: The backoff strategy from `RetryConfig`
/// - `base_delay_secs`: The base delay (typically `task_timeout_secs`)
/// - `attempt`: The consecutive failure count (1-based)
///
/// # Returns
/// The computed delay duration, capped according to the strategy's max_delay.
pub fn compute_task_backoff_delay(
    strategy: &BackoffStrategy,
    base_delay_secs: u64,
    attempt: u32,
) -> Duration {
    let attempt = attempt.max(1); // Ensure at least 1

    match strategy {
        BackoffStrategy::Constant => {
            Duration::from_secs(base_delay_secs)
        }
        BackoffStrategy::Linear { max_delay_secs } => {
            let delay = base_delay_secs.saturating_mul(attempt as u64);
            Duration::from_secs(delay.min(*max_delay_secs))
        }
        BackoffStrategy::Exponential { max_delay_secs, jitter } => {
            let exp_delay = base_delay_secs.saturating_mul(2u64.saturating_pow(attempt - 1));
            let capped = exp_delay.min(*max_delay_secs);
            if *jitter > 0.0 {
                // Apply jitter: random value in [0, jitter_amount]
                let jitter_amount = (capped as f64 * jitter) as u64;
                let random_jitter = fastrand::u64(0..=jitter_amount.max(1));
                Duration::from_secs(capped.saturating_sub(random_jitter))
            } else {
                Duration::from_secs(capped)
            }
        }
        BackoffStrategy::Fibonacci { max_delay_secs } => {
            let fib_multiplier = fibonacci(attempt);
            let delay = base_delay_secs.saturating_mul(fib_multiplier);
            Duration::from_secs(delay.min(*max_delay_secs))
        }
    }
}

/// Compute the nth Fibonacci number (1-based: fib(1)=1, fib(2)=1, fib(3)=2, ...)
fn fibonacci(n: u32) -> u64 {
    if n <= 1 {
        return 1;
    }
    let mut a: u64 = 1;
    let mut b: u64 = 1;
    for _ in 2..n {
        let next = a.saturating_add(b);
        a = b;
        b = next;
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fibonacci() {
        assert_eq!(fibonacci(1), 1);
        assert_eq!(fibonacci(2), 1);
        assert_eq!(fibonacci(3), 2);
        assert_eq!(fibonacci(4), 3);
        assert_eq!(fibonacci(5), 5);
        assert_eq!(fibonacci(6), 8);
    }

    #[test]
    fn test_constant_backoff() {
        let delay = compute_task_backoff_delay(&BackoffStrategy::Constant, 5, 1);
        assert_eq!(delay, Duration::from_secs(5));
        let delay = compute_task_backoff_delay(&BackoffStrategy::Constant, 5, 10);
        assert_eq!(delay, Duration::from_secs(5));
    }

    #[test]
    fn test_linear_backoff() {
        let strategy = BackoffStrategy::Linear { max_delay_secs: 30 };
        assert_eq!(compute_task_backoff_delay(&strategy, 2, 1), Duration::from_secs(2));
        assert_eq!(compute_task_backoff_delay(&strategy, 2, 5), Duration::from_secs(10));
        assert_eq!(compute_task_backoff_delay(&strategy, 2, 20), Duration::from_secs(30)); // capped
    }

    #[test]
    fn test_exponential_backoff_no_jitter() {
        let strategy = BackoffStrategy::Exponential { max_delay_secs: 60, jitter: 0.0 };
        assert_eq!(compute_task_backoff_delay(&strategy, 1, 1), Duration::from_secs(1));  // 1 * 2^0
        assert_eq!(compute_task_backoff_delay(&strategy, 1, 2), Duration::from_secs(2));  // 1 * 2^1
        assert_eq!(compute_task_backoff_delay(&strategy, 1, 3), Duration::from_secs(4));  // 1 * 2^2
        assert_eq!(compute_task_backoff_delay(&strategy, 1, 4), Duration::from_secs(8));  // 1 * 2^3
        assert_eq!(compute_task_backoff_delay(&strategy, 1, 7), Duration::from_secs(60)); // capped
    }

    #[test]
    fn test_fibonacci_backoff() {
        let strategy = BackoffStrategy::Fibonacci { max_delay_secs: 60 };
        assert_eq!(compute_task_backoff_delay(&strategy, 1, 1), Duration::from_secs(1));
        assert_eq!(compute_task_backoff_delay(&strategy, 1, 2), Duration::from_secs(1));
        assert_eq!(compute_task_backoff_delay(&strategy, 1, 3), Duration::from_secs(2));
        assert_eq!(compute_task_backoff_delay(&strategy, 1, 4), Duration::from_secs(3));
        assert_eq!(compute_task_backoff_delay(&strategy, 1, 5), Duration::from_secs(5));
        assert_eq!(compute_task_backoff_delay(&strategy, 1, 6), Duration::from_secs(8));
    }
}
