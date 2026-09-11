//! `bluey-protocols` — every pure wire codec used by the Bluey Tauri app.
//!
//! The app crate (`src-tauri/src`) only does I/O around these functions, so the
//! protocol logic is unit-tested on any host (the app crate itself compiles
//! only for macOS):
//!
//! * [`sse`]       — incremental Server-Sent-Events frame parser (`data:` lines, `[DONE]`).
//! * [`openai`]    — OpenAI-compatible chat/embeddings request bodies + stream chunk models.
//! * [`azure`]     — Azure OpenAI v1 (and legacy dated) URL building + deployment mapping.
//! * [`anthropic`] — Anthropic Messages API bodies + SSE event models (`input_json_delta` accumulation).
//! * [`exa`]       — Exa search request/response models.
//! * [`firecrawl`] — Firecrawl v2 scrape request/response models.
//! * [`gemini`]    — Google Gemini API (Google AI Studio): generateContent bodies + response
//!   parsing, thinking policy, error mapping, embeddings, model listing, Live API frames.
//! * [`realtime`]  — OpenAI/Azure realtime transcription WebSocket messages.
//! * [`voice_live`] — Foundry Voice Live (MAI-Transcribe) session codec + routing.
//! * [`jsonl`]     — helper/agent JSON-Lines envelope (`Request`/`Response`/`Event`).
//! * [`helper`]    — native-helper wire types → `bluey_core` type mappers (frames, OCR,
//!   AX snapshots, transcript events incl. speaker labelling).
//! * [`agent`]     — research-agent sidecar event mapping to `DeepResearchEvent`.
//! * [`panel`]     — pure panel geometry (clamping, step moves, per-display memory).
//! * [`clerk`]     — Clerk Frontend-API host derivation from a publishable key and the pure
//!   half of the Clerk browser sign-in (redirect styles, OIDC `nonce`, ID-token checks).
//! * [`oauth`]     — provider-agnostic OAuth 2.0 client primitives: PKCE, authorization URLs,
//!   redirect / manual-code parsing, token bodies and responses, JWT payload decoding, the
//!   loopback listener's HTTP bits (the runtime half is the `bluey-oauth` crate).
//! * [`request_shaper`] — the `RequestShaper` trait: a subscription provider's request
//!   fingerprint as data (headers, system blocks, drift detection), applied last by the adapter.
//! * [`fingerprints`] — the fingerprints themselves as data: the scrubbed capture format,
//!   the scrubber, per-provider rules with `VERSION` / `CAPTURED_ON`, the documented
//!   captures and the drift diff behind `bun run fingerprints:diff`.
//!
//! No Tauri, tokio, or network dependencies live here.

pub mod agent;
pub mod anthropic;
pub mod azure;
pub mod clerk;
pub mod exa;
pub mod fingerprints;
pub mod firecrawl;
pub mod gemini;
pub mod helper;
pub mod hud_menu;
pub mod jsonl;
pub mod oauth;
pub mod openai;
pub mod panel;
pub mod realtime;
pub mod request_shaper;
pub mod sse;
pub mod transcript_import;
pub mod voice_live;
