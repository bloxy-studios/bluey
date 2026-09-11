//! `bluey-oauth` — the runtime half of Bluey's OAuth client (ADR 0008 for the
//! Clerk browser sign-in, ADR 0009 for the subscription accounts).
//!
//! Platform-independent and host-tested: sockets and timers only — no Tauri,
//! no HTTP client. Callers own their `reqwest::Client`, do the token-endpoint
//! calls themselves and map failures to `BlueyError`; this crate provides the
//! pieces every browser sign-in needs and that are easy to get subtly wrong:
//!
//! * [`loopback`] — a one-shot `127.0.0.1` listener (OS-chosen or *required*
//!   fixed port) that reads exactly one request head (≤ 8 KB, 5 s), hands the
//!   request target to the caller and lets it answer with a small page;
//! * [`device`]   — RFC 8628 device-code polling with `slow_down`, expiry and
//!   cancellation;
//! * [`tokens`]   — the token set stored in the Keychain and a single-flight
//!   refresh cache (concurrent callers share one refresh);
//! * [`random`]   — state / nonce tokens and PKCE pairs from the OS RNG.
//!
//! The pure half — PKCE, URLs, redirect and manual-code parsing, token bodies
//! and responses — is `bluey_protocols::oauth`.

pub mod device;
pub mod loopback;
pub mod random;
pub mod tokens;

pub use device::{poll_device_code, DeviceFlowError, DevicePoll};
pub use loopback::{
    Accepted, LoopbackError, LoopbackListener, LoopbackPort, Responder, LOOPBACK_HOST,
    LOOPBACK_MAX_HEAD, LOOPBACK_READ_TIMEOUT,
};
pub use random::{pkce, random_bytes, random_token};
pub use tokens::{unix_now, RefreshError, TokenCache, TokenSet, DEFAULT_REFRESH_LEEWAY};
