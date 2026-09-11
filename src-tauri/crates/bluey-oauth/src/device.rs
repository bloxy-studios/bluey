//! RFC 8628 device-code polling, for providers whose loopback port may be
//! taken (the Codex CLI's fixed 1455) or that have no loopback redirect.
//!
//! The provider-specific HTTP call is the caller's closure; this function owns
//! the cadence: poll, wait `interval` (plus 5 s after every `slow_down`, as
//! the RFC prescribes), stop at the code's expiry or on cancellation.

use std::future::Future;
use std::time::Duration;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// What one poll of the token endpoint said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePoll<T, E> {
    /// `authorization_pending` — the user has not finished in the browser.
    Pending,
    /// `slow_down` — keep polling, 5 s more slowly.
    SlowDown,
    Complete(T),
    /// `access_denied`, `expired_token`, or anything else that ends the flow.
    Failed(E),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceFlowError<E> {
    /// The device code expired before the user finished.
    Expired,
    Cancelled,
    Failed(E),
}

/// Poll until the flow completes, fails, expires (`expires_in` from the
/// device-authorization response) or is cancelled. `interval` below one
/// second is raised to one second.
pub async fn poll_device_code<T, E, F, Fut>(
    interval: Duration,
    expires_in: Duration,
    cancel: &CancellationToken,
    mut poll: F,
) -> Result<T, DeviceFlowError<E>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = DevicePoll<T, E>>,
{
    let deadline = Instant::now() + expires_in;
    let mut interval = interval.max(Duration::from_secs(1));
    loop {
        if cancel.is_cancelled() {
            return Err(DeviceFlowError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(DeviceFlowError::Expired);
        }
        match poll().await {
            DevicePoll::Complete(value) => return Ok(value),
            DevicePoll::Failed(error) => return Err(DeviceFlowError::Failed(error)),
            DevicePoll::Pending => {}
            DevicePoll::SlowDown => interval += Duration::from_secs(5),
        }
        let next = (Instant::now() + interval).min(deadline);
        tokio::select! {
            _ = cancel.cancelled() => return Err(DeviceFlowError::Cancelled),
            _ = tokio::time::sleep_until(next) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use pretty_assertions::assert_eq;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn pending_polls_wait_the_interval_then_complete() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let started = Instant::now();
        let cancel = CancellationToken::new();
        let result = poll_device_code(
            Duration::from_secs(5),
            Duration::from_secs(900),
            &cancel,
            move || {
                let n = counted.fetch_add(1, Ordering::SeqCst);
                async move {
                    if n < 2 {
                        DevicePoll::Pending
                    } else {
                        DevicePoll::Complete::<_, ()>("code-1".to_string())
                    }
                }
            },
        )
        .await;
        assert_eq!(result, Ok("code-1".to_string()));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(started.elapsed(), Duration::from_secs(10));
    }

    #[tokio::test(start_paused = true)]
    async fn slow_down_adds_five_seconds_each_time() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let started = Instant::now();
        let cancel = CancellationToken::new();
        let result = poll_device_code(
            Duration::from_secs(5),
            Duration::from_secs(900),
            &cancel,
            move || {
                let n = counted.fetch_add(1, Ordering::SeqCst);
                async move {
                    match n {
                        0 => DevicePoll::SlowDown,
                        1 => DevicePoll::SlowDown,
                        _ => DevicePoll::Complete::<_, ()>(()),
                    }
                }
            },
        )
        .await;
        assert_eq!(result, Ok(()));
        // 10 s after the first slow_down, 15 s after the second.
        assert_eq!(started.elapsed(), Duration::from_secs(25));
    }

    #[tokio::test(start_paused = true)]
    async fn the_flow_expires_with_the_device_code() {
        let cancel = CancellationToken::new();
        let started = Instant::now();
        let result = poll_device_code(
            Duration::from_secs(5),
            Duration::from_secs(12),
            &cancel,
            || async { DevicePoll::<(), ()>::Pending },
        )
        .await;
        assert_eq!(result, Err(DeviceFlowError::Expired));
        assert_eq!(started.elapsed(), Duration::from_secs(12));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_poll_ends_the_flow_and_cancellation_interrupts_the_wait() {
        let cancel = CancellationToken::new();
        let result = poll_device_code(
            Duration::from_secs(5),
            Duration::from_secs(900),
            &cancel,
            || async { DevicePoll::<(), _>::Failed("access_denied") },
        )
        .await;
        assert_eq!(result, Err(DeviceFlowError::Failed("access_denied")));

        let cancel = CancellationToken::new();
        let canceller = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            canceller.cancel();
        });
        let started = Instant::now();
        let result = poll_device_code(
            Duration::from_secs(30),
            Duration::from_secs(900),
            &cancel,
            || async { DevicePoll::<(), ()>::Pending },
        )
        .await;
        assert_eq!(result, Err(DeviceFlowError::Cancelled));
        assert_eq!(started.elapsed(), Duration::from_secs(2));
    }

    #[tokio::test(start_paused = true)]
    async fn sub_second_intervals_are_raised_to_one_second() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let started = Instant::now();
        let cancel = CancellationToken::new();
        let _ = poll_device_code(
            Duration::from_millis(10),
            Duration::from_secs(900),
            &cancel,
            move || {
                let n = counted.fetch_add(1, Ordering::SeqCst);
                async move {
                    if n == 0 {
                        DevicePoll::Pending
                    } else {
                        DevicePoll::Complete::<_, ()>(())
                    }
                }
            },
        )
        .await;
        assert_eq!(started.elapsed(), Duration::from_secs(1));
    }
}
