//! `bluey-fingerprints` — the fingerprint capture harness (ADR 0009, PR 2b).
//!
//! Three jobs, all reachable through `bun run fingerprints:*` from the repository
//! root (see `docs/PROVIDER_ACCOUNTS.md › Re-capture runbook`):
//!
//! * [`proxy`] — a local HTTP/1.1 server the official CLI is pointed at through its
//!   base-URL knob (`ANTHROPIC_BASE_URL=http://127.0.0.1:<port>`); every request is
//!   forwarded to the real upstream, the response is streamed back, and the exchange
//!   is scrubbed ([`bluey_protocols::fingerprints::scrub`]) *before* it is written
//!   to `tests/fixtures/fingerprints/<provider>/captures/`.
//! * [`har`] — the same capture from a HAR export of a MITM proxy, for clients without
//!   a base-URL knob (the Antigravity Electron app).
//! * [`store`] — where captures, blessed goldens and documented captures live, and
//!   `bless` (a reviewed capture becomes `golden/<endpoint>.json`).
//!
//! The diff itself is pure and lives in `bluey_protocols::fingerprints::diff`; the
//! binary (`src/main.rs`) only wires files to it and renders the report.
//!
//! Nothing here is compiled into the app: the app keeps `accounts_probe_fingerprint`,
//! which PR 3a–3c fill in with the same capture format.

pub mod har;
pub mod http1;
pub mod proxy;
pub mod store;

/// Install the `ring` crypto provider for rustls once per process (reqwest is built
/// with `rustls-no-provider`). Safe to call repeatedly.
pub fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// RFC 3339 UTC timestamp for `Capture::captured_at`.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
