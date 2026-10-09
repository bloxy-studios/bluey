# ADR 0012 — Panics: unwind so a crashing parser can be refused; abort on every other panic

**Status:** accepted · **Date:** 2026-09-29 · companion: `src-tauri/crates/bluey-core/src/panic.rs`,
`src-tauri/src/logging/panic_hook.rs`, `docs/audits/BLUEY_DEEP_AUDIT_2026-09-28.md` (CRIT-002)

## Context
Release builds used `panic = "abort"`. `pdf-extract` panics (`panic!`, `unwrap`) on many
malformed or unusual PDFs, so adding one bad résumé to a mode took the whole app down: the
helper, the listening run and the answer in progress went with it (CRIT-002).

Switching the profile to `panic = "unwind"` lets the parser's panic be caught, but on its own
it changes what every other panic does. A panic in a Tauri command or a spawned task would
unwind that one task and leave the rest of the app running half-updated. A Finder-launched app
discards stderr, so nothing would record that it happened (the final review raised this as
panic-unwind-no-hook).

## Decision
1. **Unwind, but only to contain named third-party code.** `bluey_core::panic::contain(f)`
   wraps `catch_unwind` and marks the current thread as containing (a thread-local depth that
   nests). Document parsing runs PDF extraction inside it. A panic there becomes
   `storage.parse` ("PDF text extraction failed"), and the panic payload is not echoed because
   it can quote document text. DOCX entries are read with a decompressed-size cap.
2. **The profile cannot drift back.** `documents/parse.rs` has
   `#[cfg(not(panic = "unwind"))] compile_error!(…)`, so an `abort` profile fails to build
   instead of silently bringing the crash back.
3. **Every other panic is fatal and logged.** Bootstrap installs a panic hook after logging is
   initialised (`logging/panic_hook.rs`). A contained panic is logged at `warn` with its
   location and thread, and the hook returns. Any other panic is logged at `error` with
   location and thread, never the payload. The hook then calls the previous hook and calls
   `std::process::abort()`, which leaves a macOS crash report as `panic = "abort"` did.

## Consequences
- One malformed document is refused with a message; it no longer ends the session.
- Outside `contain`, panics behave as before (the process aborts), and the log now says where.
- Code run inside `contain` must not leave shared state half-written when it unwinds. The
  parser takes a byte slice and returns owned text, so it holds no shared state; new callers
  of `contain` need the same property.
- Unwind tables make the binary slightly larger.
- Tests: `bluey-core` pins what counts as contained (nesting, leaving the scope after an
  unwind). A child-process test in the app crate checks that an uncontained panic aborts and
  that the log names the location without the payload.

## Alternatives considered
- **Keep `abort` and parse in a separate process.** This isolates the parser better, including
  against memory blow-ups, but needs a helper process, an IPC contract and lifecycle handling
  for a rare failure. Not chosen now. Revisit if more untrusted parsers are added.
- **Unwind everywhere and catch at command boundaries.** Rejected: an unwound task can leave
  managers and the state machine inconsistent while the app keeps running.
- **Replace `pdf-extract`.** Possible later; containment is needed regardless, because any
  third-party parser can panic.
