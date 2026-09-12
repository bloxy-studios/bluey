# CI / release workflows

The active definitions are in `.github/workflows/`. Install copies remain under
`docs/ci/workflows/` for maintainers whose integration token lacks GitHub's `workflow` scope.
The release copy must match the active workflow (a portable test enforces this), so installing
it cannot silently restore the old ungated build-only workflow. Review any copied CI changes:

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

Runs the macOS arm64/x64 matrix with **Bun 1.4.2**, frozen installs and the existing helper/
agent chain. Default manual dispatch remains `publish=false` (developer `.app`/`.dmg`
artifacts only), retaining `research_backend=gemini|claude`.

Tag pushes, or manual `publish=true` + an **existing** `release_tag`, use a separate strict
publication path: all six signing/notarization secrets required before toolchain/build;
version/tag/commit consistency; real Apple signature/notarization/staple/Gatekeeper checks on
the app and DMG; complete current-run/current-attempt artifact pair; independent native
re-verification after download; actual `bluey-downloads.json` + `SHA256SUMS`; create a new
**draft**, upload/read back all four assets, then finalize stable or prerelease. Existing
drafts/published releases are never overwritten and tags are never created or forced.

Default permissions are `contents: read`; only the final publish job has `contents: write`.
No release job executes on pull requests. The separate `release-scripts` CI job runs Python
stdlib tests and Bash syntax checks without secrets, including on PRs.

See [the complete owner release runbook](../RELEASING.md) for protected environment/tag setup,
required secrets, public Clerk variables, optional desktop OAuth value, manual dispatch,
re-run/partial-draft handling and the native validation boundary. No provider API keys are
needed in CI tests; manual keyed smoke tests remain in `docs/TESTING.md`.
