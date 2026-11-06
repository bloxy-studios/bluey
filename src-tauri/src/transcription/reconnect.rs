//! Reconnect policy shared by the streaming (WebSocket) providers.
//!
//! A lost connection is re-opened with capped exponential backoff until it
//! succeeds, the session is closed, the service rejects the configuration (a
//! bad key must never be retried in a loop), or the outage has cost
//! [`MAX_OUTAGE_ATTEMPTS`] attempts (the manager then re-opens the source after
//! a cool-down or moves it to Apple Speech). Liveness is not inferred from
//! socket errors alone: sends time out, and a read watchdog probes with a ping
//! — then reconnects — when speech went out but nothing came back.
//!
//! Audio is not thrown away across a reconnect: what was sent since the last
//! final and what arrives while reconnecting is kept in a [`Replay`] buffer
//! (about ten seconds) and sent to the new connection first, so a question
//! asked during a network blip is still transcribed.

use std::collections::VecDeque;
use std::future::Future;
use std::time::Duration;

use bluey_core::types::AudioSource;
use bluey_core::{BlueyError, BlueyErrorKind};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::{EventSink, PcmChunk, TranscriptionEvent};

/// A send that has not completed by then is treated as a dead socket.
pub(crate) const SEND_TIMEOUT: Duration = Duration::from_secs(5);
/// Speech was forwarded but the server has said nothing for this long: probe.
pub(crate) const READ_WATCHDOG: Duration = Duration::from_secs(15);
/// After a probe (a WebSocket ping), how long any frame may take to arrive
/// before the socket counts as stalled.
pub(crate) const PROBE_GRACE: Duration = Duration::from_secs(5);
/// Reconnect attempts one outage may cost before the session gives up.
pub(crate) const MAX_OUTAGE_ATTEMPTS: u32 = 8;
/// Chunks kept for replay — about ten seconds at 200 ms chunks.
pub(crate) const REPLAY_CHUNKS: usize = 50;
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
///
/// Silence from the server is not proof of a dead socket — typing or music
/// trips the helper's energy VAD and the service rightly transcribes nothing —
/// so the first expiry asks for a probe (a ping; its pong is a frame) and
/// only a probe that goes unanswered for [`PROBE_GRACE`] counts as stalled.
#[derive(Debug, Default)]
pub(crate) struct Watchdog {
    waiting_since: Option<Instant>,
    probed: bool,
}

/// What an expired [`Watchdog`] asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WatchdogVerdict {
    /// Send a ping and wait [`PROBE_GRACE`] for any frame.
    Probe,
    /// The probe went unanswered: reconnect.
    Stalled,
}

impl Watchdog {
    pub(crate) fn sent_speech(&mut self) {
        self.waiting_since.get_or_insert_with(Instant::now);
    }

    /// Any frame (a transcript, a pong) arrived: the socket is alive.
    pub(crate) fn heard(&mut self) {
        self.waiting_since = None;
        self.probed = false;
    }

    /// Resolves when the server has been silent for [`READ_WATCHDOG`] after
    /// speech was sent (or for [`PROBE_GRACE`] after a probe); never while
    /// nothing is awaited.
    pub(crate) async fn expired(&self) {
        let window = if self.probed {
            PROBE_GRACE
        } else {
            READ_WATCHDOG
        };
        match self.waiting_since {
            Some(since) => tokio::time::sleep_until(since + window).await,
            None => std::future::pending().await,
        }
    }

    /// The watchdog expired: probe first, stalled only after a failed probe.
    pub(crate) fn verdict(&mut self) -> WatchdogVerdict {
        if self.probed {
            return WatchdogVerdict::Stalled;
        }
        self.probed = true;
        self.waiting_since = Some(Instant::now());
        WatchdogVerdict::Probe
    }
}

/// Recent audio a new connection is given first after a reconnect: the chunks
/// sent since the last final (the old socket may never have transcribed them)
/// and the ones that arrived while reconnecting. Bounded: the oldest go first.
#[derive(Debug, Default)]
pub(crate) struct Replay {
    chunks: VecDeque<PcmChunk>,
}

impl Replay {
    pub(crate) fn push(&mut self, chunk: PcmChunk) {
        if self.chunks.len() == REPLAY_CHUNKS {
            self.chunks.pop_front();
        }
        self.chunks.push_back(chunk);
    }

    /// A final arrived: everything before it is transcribed.
    pub(crate) fn clear(&mut self) {
        self.chunks.clear();
    }

    /// The buffered chunks, oldest first (they stay buffered until a final).
    pub(crate) fn pending(&self) -> Vec<PcmChunk> {
        self.chunks.iter().cloned().collect()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }
}

/// How [`reopen`] ended.
pub(crate) enum Reopened<C> {
    Open(C),
    /// The session was closed while reconnecting.
    Closed,
    Fatal(BlueyError),
    /// [`MAX_OUTAGE_ATTEMPTS`] reconnects failed; the error is the last one.
    GaveUp(BlueyError),
}

/// Open a connection, retrying with [`backoff`] until it succeeds, the session
/// is closed, the error is fatal, or [`MAX_OUTAGE_ATTEMPTS`] attempts failed.
/// `attempt` is how many reconnects this outage has already cost (0 connects
/// at once). Audio that arrives meanwhile goes into `replay` for the new
/// connection, and `Close` is honoured even mid-connect so closing never
/// waits out an outage.
pub(crate) async fn reopen<C, F, Fut>(
    rx: &mut mpsc::Receiver<Command>,
    outage: &mut Outage,
    mut attempt: u32,
    replay: &mut Replay,
    mut connect: F,
) -> Reopened<C>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<C, BlueyError>>,
{
    let mut last_error = None;
    loop {
        if attempt > MAX_OUTAGE_ATTEMPTS {
            return Reopened::GaveUp(last_error.unwrap_or_else(|| {
                BlueyError::network("connect", "the transcription service stayed unreachable")
            }));
        }
        if attempt > 0 {
            tokio::select! {
                () = closed(rx, replay) => return Reopened::Closed,
                () = tokio::time::sleep(backoff(attempt)) => {}
            }
        }
        let result = tokio::select! {
            () = closed(rx, replay) => return Reopened::Closed,
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
                outage.degraded(error.clone()).await;
                last_error = Some(error);
                attempt = attempt.saturating_add(1);
            }
        }
    }
}

/// Resolves once the session is closed, buffering audio for replay until then.
async fn closed(rx: &mut mpsc::Receiver<Command>, replay: &mut Replay) {
    while let Some(Command::Audio(chunk)) = rx.recv().await {
        replay.push(chunk);
    }
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
        let reopened = reopen(&mut rx, &mut notice, 0, &mut Replay::default(), || {
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
        let reopened: Reopened<()> =
            reopen(&mut rx, &mut notice, 0, &mut Replay::default(), || {
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
        let reopened: Reopened<()> =
            reopen(&mut rx, &mut notice, 5, &mut Replay::default(), || async {
                Ok(())
            })
            .await;

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
        assert_eq!(
            watchdog.verdict(),
            WatchdogVerdict::Probe,
            "probe before reconnecting"
        );
        assert_eq!(
            watchdog.verdict(),
            WatchdogVerdict::Stalled,
            "an unanswered probe"
        );

        watchdog.heard();
        assert!(!watchdog.probed, "a pong clears the probe");
        let answered = tokio::time::timeout(Duration::from_millis(20), watchdog.expired()).await;
        assert!(answered.is_err());
    }

    fn chunk(n: u64) -> PcmChunk {
        PcmChunk {
            source: AudioSource::Microphone,
            base64: String::new(),
            sample_rate: 16_000,
            start_ms: n * 200,
            end_ms: n * 200 + 200,
            is_speech: true,
        }
    }

    /// Audio said while reconnecting is kept for the new connection, bounded.
    #[tokio::test]
    async fn audio_during_an_outage_is_kept_for_replay() {
        let (tx, mut rx) = mpsc::channel(128);
        let (mut notice, _events) = outage();
        for n in 0..(REPLAY_CHUNKS as u64 + 5) {
            tx.send(Command::Audio(chunk(n))).await.unwrap();
        }
        tx.send(Command::Close).await.unwrap();
        let mut replay = Replay::default();
        let reopened: Reopened<()> = reopen(&mut rx, &mut notice, 1, &mut replay, || {
            std::future::pending()
        })
        .await;
        assert!(matches!(reopened, Reopened::Closed));
        let pending = replay.pending();
        assert_eq!(pending.len(), REPLAY_CHUNKS);
        assert_eq!(pending[0].start_ms, 5 * 200, "the oldest are dropped first");
        replay.clear();
        assert!(replay.is_empty());
    }

    /// An outage that has used its attempts gives up instead of retrying forever.
    #[tokio::test]
    async fn an_endless_outage_gives_up() {
        let (_tx, mut rx) = mpsc::channel(8);
        let (mut notice, _events) = outage();
        let calls = Cell::new(0);
        let reopened: Reopened<()> = reopen(
            &mut rx,
            &mut notice,
            MAX_OUTAGE_ATTEMPTS + 1,
            &mut Replay::default(),
            || {
                calls.set(calls.get() + 1);
                async { Ok(()) }
            },
        )
        .await;
        assert_eq!(calls.get(), 0, "no attempt past the budget");
        match reopened {
            Reopened::GaveUp(error) => assert_eq!(error.code, "network.connect"),
            _ => panic!("expected the outage to give up"),
        }
    }
}
