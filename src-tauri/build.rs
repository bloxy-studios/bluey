//! Build script: Tauri codegen plus the compile-time defaults for the *public*
//! Clerk settings.
//!
//! Before ADR 0008 the WebView read `VITE_CLERK_PUBLISHABLE_KEY` from
//! `import.meta.env`, i.e. Vite baked it into the bundle at build time from the
//! repository's `.env` / `.env.local`. The Rust side now owns sign-in, so the
//! same files are read here and the allowlisted values below are embedded as
//! `BLUEY_BAKED_<NAME>` for `option_env!` — installed builds carry their Clerk
//! configuration without any file next to the binary. At run time the process
//! environment (including `.env` / `.env.local` loaded at boot) still wins.
//!
//! Only public identifiers are ever baked: the allowlist is the whole point.
//! API keys stay in the Keychain / runtime environment and never enter the
//! binary. The one non-Clerk entry, `BLUEY_ANTIGRAVITY_CLIENT_SECRET`, is
//! Google's *desktop-app* OAuth client secret — public in the shipped
//! Antigravity app, not a user secret (`docs/SECURITY.md`) — which the repo
//! deliberately does not commit; baking it from `.env` / `.env.local` is what
//! lets *Connect Google AI* open the browser in dev and installed builds alike
//! (`docs/PROVIDER_ACCOUNTS.md › Google AI`). Values are never printed.

use std::collections::BTreeMap;
use std::path::PathBuf;

#[allow(dead_code)]
#[path = "src/dotenv.rs"]
mod dotenv;

/// Public, per-instance settings compiled in from `.env` / `.env.local`.
const BAKED_SETTINGS: &[&str] = &[
    "VITE_CLERK_PUBLISHABLE_KEY",
    "VITE_CLERK_FRONTEND_API_URL",
    "BLUEY_CLERK_OAUTH_CLIENT_ID",
    "BLUEY_CLERK_ACCOUNT_PORTAL_URL",
    // Read by `accounts::antigravity::client_secret` as `BLUEY_BAKED_BLUEY_ANTIGRAVITY_CLIENT_SECRET`.
    "BLUEY_ANTIGRAVITY_CLIENT_SECRET",
];

fn main() {
    bake_public_settings();
    tauri_build::build()
}

fn bake_public_settings() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.clone());

    // `.env` first, `.env.local` on top — the same precedence as Vite and Bun.
    let mut from_files: BTreeMap<String, String> = BTreeMap::new();
    for name in [".env", ".env.local"] {
        let path = root.join(name);
        let Ok(contents) = std::fs::read_to_string(&path) else {
            // No `rerun-if-changed` for a missing file: Cargo would treat it as
            // always stale and re-run this script (and rebuild) on every build.
            continue;
        };
        println!("cargo:rerun-if-changed={}", path.display());
        for (key, value) in dotenv::parse(&contents) {
            if BAKED_SETTINGS.contains(&key.as_str()) {
                from_files.insert(key, value);
            }
        }
    }

    for name in BAKED_SETTINGS {
        // The build environment (CI variables, an `export` in the shell) wins
        // over the files.
        println!("cargo:rerun-if-env-changed={name}");
        let value = std::env::var(name)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| from_files.get(*name).cloned());
        if let Some(value) = value {
            println!("cargo:rustc-env=BLUEY_BAKED_{name}={value}");
        }
    }
}
