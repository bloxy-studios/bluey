//! Reconnect policy shared by the streaming (WebSocket) providers.
//!
//! A lost connection is re-opened with capped exponential backoff until it
//! succeeds, the session is closed, or the service rejects the configuration
//! (a bad key must never be retried in a loop). Liveness is not inferred from
//! socket errors alone: sends time out, and a read watchdog reconnects when
//! speech went out but nothing came back.

use std::future::Future;
use std::time::Duration;

use bluey_core::types::AudioSource;
use bluey_core::{BlueyError, BlueyErrorKind};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::{EventSink, PcmChunk, TranscriptionEvent};

/// A send that has not completed by then is treated as a dead socket.
pub(crate) const SEND_TIMEOUT: Duration = Duration::from_secs(5);
/// Speech was forwarded but the server has said nothing for this long.
pub(crate) const READ_WATCHDOG: Duration = Duration::from_secs(15);
const BACKOFF_BASE: Duration = Duration::from_millis(500);
const BACKOFF_CAP: Duration = Duration::from_secs(8);

/// What the session hands its worker.
pub(crate) enum Command {
    Audio(PcmChunk),
    Close,
}

/// Configuration errors end the session; everything else is worth a reconnect.
pub(crate) fn is_fatal(error: &BlueyError) -> bool {
    error.kind == BlueyErrorKind::Configuration
}

/// Delay before reconnect attempt `attempt` (1-based): 0.5 s doubling to 8 s.
pub(crate) fn backoff(attempt: u32) -> Duration {
    BACKOFF_BASE
        .checked_mul(2u32.saturating_pow(attempt.saturating_sub(1)))
        .unwrap_or(BACKOFF_CAP)
        .min(BACKOFF_CAP)
}

/// Reports an outage to the manager once, and its end.
pub(crate) struct Outage {
    source: AudioSource,
    sink: EventSink,
    announced: bool,
}

impl Outage {
    pub(crate) fn new(source: AudioSource, sink: EventSink) -> Self {
        Self {
            source,
            sink,
            announced: false,
        }
    }

    /// A reconnect attempt failed: tell the manager, once per outage. A blip
    /// that the first attempt repairs is never announced.
    async fn degraded(&mut self, error: BlueyError) {
        if std::mem::replace(&mut self.announced, true) {
            return;
        }
        let event = TranscriptionEvent::Degraded {
            source: self.source,
            error,
        };
        let _ = self.sink.send(event).await;
    }

    async fn recovered(&mut self) {
        if std::mem::take(&mut self.announced) {
            let _ = self
                .sink
                .send(TranscriptionEvent::Recovered {
                    source: self.source,
                })
                .await;
        }
    }
}

/// Speech went out and nothing has come back since.
#[derive(Debug, Default)]
pub(crate) struct Watchdog {
    waiting_since: Option<Instant>,
}

impl Watchdog {
    pub(crate) fn sent_speech(&mut self) {
        self.waiting_since.get_or_insert_with(Instant::now);
    }

    pub(crate) fn heard(&mut self) {
        self.waiting_since = None;
    }

    /// Resolves when the server has been silent for [`READ_WATCHDOG`] after
    /// speech was sent; never while nothing is awaited.
    pub(crate) async fn expired(&self) {
        match self.waiting_since {
            Some(since) => tokio::time::sleep_until(since + READ_WATCHDOG).await,
            None => std::future::pending().await,
        }
    }
}

/// How [`reopen`] ended.
pub(crate) enum Reopened<C> {
    Open(C),
    /// The session was closed while reconnecting.
    Closed,
    Fatal(BlueyError),
}

/// Open a connection, retrying with [`backoff`] until it succeeds, the session
/// is closed, or the error is fatal. `attempt` is how many reconnects this
/// outage has already cost (0 connects at once). Audio that arrives meanwhile
/// is dropped (late real-time audio is useless), and `Close` is honoured even
/// mid-connect so closing never waits out an outage.
pub(crate) async fn reopen<C, F, Fut>(
    rx: &mut mpsc::Receiver<Command>,
    outage: &mut Outage,
    mut attempt: u32,
    mut connect: F,
) -> Reopened<C>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<C, BlueyError>>,
{
    loop {
        if attempt > 0 {
            tokio::select! {
                () = closed(rx) => return Reopened::Closed,
                () = tokio::time::sleep(backoff(attempt)) => {}
            }
        }
        let result = tokio::select! {
            () = closed(rx) => return Reopened::Closed,
            result = connect() => result,
        };
        match result {
            Ok(conn) => {
                outage.recovered().await;
                return Reopened::Open(conn);
            }
            Err(error) if is_fatal(&error) => return Reopened::Fatal(error),
            Err(error) => {
                tracing::warn!(attempt, error = %error, "transcription reconnect failed; retrying");
                outage.degraded(error).await;
                attempt = attempt.saturating_add(1);
            }
        }
    }
}

/// Resolves once the session is closed, discarding audio until then.
async fn closed(rx: &mut mpsc::Receiver<Command>) {
    while let Some(Command::Audio(_)) = rx.recv().await {}
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn outage() -> (Outage, mpsc::Receiver<TranscriptionEvent>) {
        let (sink, events) = mpsc::channel(8);
        (Outage::new(AudioSource::Microphone, sink), events)
    }

    #[test]
    fn backoff_grows_and_caps_at_eight_seconds() {
        assert_eq!(backoff(1), Duration::from_millis(500));
        assert_eq!(backoff(2), Duration::from_secs(1));
        assert_eq!(backoff(5), Duration::from_secs(8));
        assert_eq!(backoff(40), Duration::from_secs(8));
    }

    /// LIVE-004: a network outage is retried until the service is back —
    /// announced once, then cleared — instead of ending the session.
    #[tokio::test]
    async fn an_outage_is_retried_until_the_service_is_back() {
        let (_tx, mut rx) = mpsc::channel(8);
        let (mut notice, mut events) = outage();
        let calls = Cell::new(0);
        let reopened = reopen(&mut rx, &mut notice, 0, || {
            calls.set(calls.get() + 1);
            let n = calls.get();
            async move {
                match n {
                    1 | 2 => Err(BlueyError::network("connect", "offline")),
                    _ => Ok(n),
                }
            }
        })
        .await;

        assert!(matches!(reopened, Reopened::Open(3)));
        assert!(matches!(
            events.try_recv(),
            Ok(TranscriptionEvent::Degraded { .. })
        ));
        assert!(matches!(
            events.try_recv(),
            Ok(TranscriptionEvent::Recovered { .. })
        ));
        assert!(events.try_recv().is_err(), "one notice per outage");
    }

    #[tokio::test]
    async fn a_rejected_configuration_is_never_retried() {
        let (_tx, mut rx) = mpsc::channel(8);
        let (mut notice, mut events) = outage();
        let calls = Cell::new(0);
        let reopened: Reopened<()> = reopen(&mut rx, &mut notice, 0, || {
            calls.set(calls.get() + 1);
            async { Err(BlueyError::configuration("api_key_invalid", "bad key")) }
        })
        .await;

        assert!(matches!(reopened, Reopened::Fatal(_)));
        assert_eq!(calls.get(), 1);
        assert!(events.try_recv().is_err());
    }

    /// Closing mid-outage returns at once instead of waiting out the backoff
    /// (the session would otherwise abort the worker).
    #[tokio::test]
    async fn close_during_backoff_ends_the_outage_promptly() {
        let (tx, mut rx) = mpsc::channel(8);
        let (mut notice, _events) = outage();
        tx.send(Command::Close).await.unwrap();
        let started = std::time::Instant::now();
        let reopened: Reopened<()> = reopen(&mut rx, &mut notice, 5, || async { Ok(()) }).await;

        assert!(matches!(reopened, Reopened::Closed));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    /// LIVE-013: the watchdog only runs while speech awaits an answer.
    #[tokio::test]
    async fn the_watchdog_waits_only_for_unanswered_speech() {
        let mut watchdog = Watchdog::default();
        let idle = tokio::time::timeout(Duration::from_millis(20), watchdog.expired()).await;
        assert!(idle.is_err(), "nothing sent, nothing awaited");

        watchdog.sent_speech();
        let first = watchdog.waiting_since;
        watchdog.sent_speech();
        assert_eq!(
            watchdog.waiting_since, first,
            "measured from the first unanswered chunk"
        );

        watchdog.waiting_since = Some(Instant::now() - READ_WATCHDOG);
        let stalled = tokio::time::timeout(Duration::from_millis(20), watchdog.expired()).await;
        assert!(stalled.is_ok(), "silent past the window");

        watchdog.heard();
        let answered = tokio::time::timeout(Duration::from_millis(20), watchdog.expired()).await;
        assert!(answered.is_err());
    }
}
