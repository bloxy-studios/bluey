//! Rust ↔ TypeScript parity fixtures (TEST-014).
//!
//! The mock transport and the TS tests import these JSON files instead of
//! keeping hand-written copies of Rust data. This test fails when a file
//! drifts from Rust; after changing a built-in mode or a default, regenerate
//! with:
//!
//! ```sh
//! BLUEY_UPDATE_FIXTURES=1 cargo test -p bluey-core --test ts_fixtures
//! ```

use std::path::PathBuf;

/// Timestamps in the fixtures (the mock re-stamps them).
const FIXED_TIME: &str = "2026-01-01T00:00:00.000Z";

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/rust")
        .join(name)
}

fn check_fixture(name: &str, value: &impl serde::Serialize) {
    let expected = serde_json::to_string_pretty(value).expect("serialize fixture") + "\n";
    let path = fixture_path(name);
    if std::env::var_os("BLUEY_UPDATE_FIXTURES").is_some() {
        std::fs::create_dir_all(path.parent().expect("fixture dir")).expect("create fixture dir");
        std::fs::write(&path, &expected).expect("write fixture");
        return;
    }
    let actual = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        actual == expected,
        "{} drifted from Rust; regenerate with \
         `BLUEY_UPDATE_FIXTURES=1 cargo test -p bluey-core --test ts_fixtures`",
        path.display()
    );
}

#[test]
fn built_in_modes_fixture_matches_rust() {
    check_fixture(
        "built-in-modes.json",
        &bluey_core::modes::built_in_modes(FIXED_TIME),
    );
}
