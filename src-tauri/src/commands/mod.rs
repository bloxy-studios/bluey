//! Tauri command surface — one submodule per section of
//! `src/lib/tauri/commands.ts`. Every function here is registered in
//! `generate_handler!` in `lib.rs` (checked by
//! `tests/integration/command-surface.test.ts`). Commands only validate,
//! delegate to the managers in [`crate::state::AppCore`] and map results onto
//! the typed contract; no business logic lives here.

pub mod accounts;
pub mod ai;
pub mod app;
pub mod audio;
pub mod auth;
pub mod capture;
pub mod context;
pub mod data;
pub mod dev;
pub mod documents;
pub mod hud_menu;
pub mod modes;
pub mod panel;
pub mod permissions;
pub mod research;
pub mod responses;
pub mod sessions;
pub mod settings;
pub mod shortcuts;
