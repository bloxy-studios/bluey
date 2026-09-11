//! Bluey's monotonic clock for the ⌘↵ fast-path trace (ADR 0010 §2): milliseconds
//! since the process started, never wall time. Every Rust-side stamp of a
//! `LatencyTrace` comes from here; the WebView cannot read this clock, so it
//! reports offsets against an anchor Rust stamped (`bluey_core::latency`).

use std::sync::OnceLock;
use std::time::Instant;

static ORIGIN: OnceLock<Instant> = OnceLock::new();

/// Fix the origin (called first thing at bootstrap; later calls are no-ops).
pub fn init() {
    let _ = ORIGIN.get_or_init(Instant::now);
}

/// Milliseconds since the origin, with sub-millisecond precision.
pub fn mono_ms() -> f64 {
    ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_only_moves_forward() {
        init();
        let a = mono_ms();
        let b = mono_ms();
        assert!(b >= a);
        assert!(a >= 0.0);
    }
}
