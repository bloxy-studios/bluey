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
//! * [`realtime`]  — OpenAI/Azure realtime transcription WebSocket messages.
//! * [`voice_live`] — Foundry Voice Live (MAI-Transcribe) session codec + routing.
//! * [`jsonl`]     — helper/agent JSON-Lines envelope (`Request`/`Response`/`Event`).
//! * [`helper`]    — native-helper wire types → `bluey_core` type mappers (frames, OCR,
//!   AX snapshots, transcript events incl. speaker labelling).
//! * [`agent`]     — research-agent sidecar event mapping to `DeepResearchEvent`.
//! * [`panel`]     — pure panel geometry (clamping, step moves, per-display memory).
//! * [`clerk`]     — Clerk Frontend-API host derivation from a publishable key.
//!
//! No Tauri, tokio, or network dependencies live here.

pub mod agent;
pub mod anthropic;
pub mod azure;
pub mod clerk;
pub mod exa;
pub mod firecrawl;
pub mod helper;
pub mod jsonl;
pub mod openai;
pub mod panel;
pub mod realtime;
pub mod sse;
pub mod voice_live;
