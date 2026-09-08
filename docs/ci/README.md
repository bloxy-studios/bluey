# CI / release workflows

The GitHub Actions definitions live here instead of `.github/workflows/` because the
integration token that pushes Bluey's pull requests does not carry the `workflow` scope
(GitHub rejects such pushes with 403). A maintainer installs them once:

```bash
bash scripts/install-workflows.sh     # copies docs/ci/workflows/*.yml → .github/workflows/
git add .github/workflows && git commit -m "ci: add workflows"
```

## `ci.yml` — every push to `main` and every pull request

| Job | Runner | What it runs |
|---|---|---|
| Frontend | ubuntu | `bun install --frozen-lockfile`, `typecheck`, `lint`, `test`, `build` |
| Rust | ubuntu | `scripts/check-rust.sh --darwin` (rustfmt, `cargo test` + clippy `-D warnings` for `bluey-core` / `bluey-storage` / `bluey-protocols`, `cargo check` of the app crate for `aarch64-apple-darwin` with the stub C compiler) plus `cargo clippy --all-targets -D warnings` of the app crate on the macOS target so its unit tests type-check too. Placeholder sidecar files satisfy tauri-build's `externalBin` check. |
| macOS | macos-14 | Swift helper build, **lite** agent sidecar build, `cargo test --features dev-tools` for the app crate (the only place its unit tests can run), app-crate clippy |

The Rust job does not link anything for macOS — see `docs/DEVELOPMENT.md` › *Working without macOS*.

## `release.yml` — tags `v*` (or manual dispatch)

Runs `scripts/release.sh` per target (`aarch64-apple-darwin`, `x86_64-apple-darwin`) on macos-14
and uploads the `.dmg` / `.app` bundles as artifacts.

- `research_backend` (dispatch input, default `gemini`) selects the sidecar variant baked into
  the build: **lite** (Gemini function calling over `@google/genai`, no embedded Claude CLI) or
  **full** (`RESEARCH_BACKEND=claude`, embeds the Claude CLI).
- Signing / notarization are optional and driven by secrets: `APPLE_CERTIFICATE_P12` (base64
  `.p12`) + `APPLE_CERTIFICATE_PASSWORD` to import the identity, `APPLE_SIGNING_IDENTITY`,
  and `APPLE_ID` + `APPLE_PASSWORD` (app-specific) + `APPLE_TEAM_ID` for notarization. Without
  them the build is ad-hoc signed and not distributable.

No API keys are needed in CI: nothing talks to a provider during the build or the tests
(the Gemini smoke tests in `docs/TESTING.md` are a manual, keyed step).
