//! `bluey-core` — platform-independent domain layer shared by the Tauri app and
//! the storage crate.
//!
//! Contents (see module docs):
//! * [`types`]  — serde data model mirroring `src/lib/types/*.ts` **exactly** (camelCase).
//! * [`error`]  — the typed [`error::BlueyError`] every subsystem returns.
//! * [`state`]  — the explicit application state machine.
//! * [`events`] — the central event enum + names used on the bus.
//! * [`router`] — task/latency/vision aware model routing policy.
//! * [`budget`] — context token budgeting & trimming.
//! * [`shortcuts`] — default bindings, accelerator parsing, conflict detection.
//! * [`modes`]  — built-in mode definitions as data.
//! * [`presets`] — provider presets (reserved ids, recommended models) + `.env` import planning.
//! * [`session`] — session lifecycle rules.
//! * [`context`] — native context snapshot assembly helpers (size limits, adapters).
//! * [`text`]   — token estimation, chunking, dedupe utilities.
//!
//! This crate has **no** Tauri, HTTP or OS dependencies so it compiles and tests on any host.

pub mod error;
pub mod types;

pub mod accounts;
pub mod budget;
pub mod context;
pub mod events;
pub mod modes;
pub mod presets;
pub mod router;
pub mod session;
pub mod shortcuts;
pub mod state;
pub mod text;

pub use error::{BlueyError, BlueyErrorKind, BlueyResult};
pub use types::*;

/// Current time as an RFC 3339 string with millisecond precision (the format used everywhere).
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// New random identifier with a short prefix, e.g. `ses_3f9a…`.
pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}
