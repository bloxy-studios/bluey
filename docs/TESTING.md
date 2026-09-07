# Testing

## Automated

| Layer | Tool | Command | Covers |
|---|---|---|---|
| Rust core | `cargo test -p bluey-core` | `bun run test:rust` | state machine matrix, event names ⇄ TS list, model routing & fallbacks, token budgeting, snapshot trimming, adapters, built-in modes, shortcut parsing/conflicts, session rules, text utils |
| Rust storage | `cargo test -p bluey-storage` | `bun run test:rust` | migrations, every repository, FTS sync & session search, document parsing (PDF/DOCX/TXT/MD), chunking, keyword/semantic retrieval, retention really deleting |
| TypeScript unit | Vitest | `bun run test` | context fusion, budget, intent, prompt builder, code-fence buffering, generation gate, optimizer, structured parsing, classifier, speaker labels, research router + privacy scrubbing, summaries, stores |
| TypeScript integration | Vitest + fake transport | `bun run test` | capture → context → request, transcript → detection → prepare/take, stale-request protection, retrieval scoping, session → summary, command-surface parity |
| UI | Vitest + Testing Library + MockTransport | `bun run test` | HUD per app state, keybind recording, mode editor, settings toggles, response rendering/copy, onboarding flow |
| Agent sidecar | Vitest | `cd sidecars/agent && bun run test` | protocol, tool mapping (mocked fetch), allow-lists, cancellation, mock-mode end-to-end |
| Swift helper | `swift test` (macOS) | `bash scripts/test-helper.sh` | dHash/VAD/envelope/ordering logic |
| Type/lint | tsc, ESLint, clippy, rustfmt | `bun run typecheck && bun run lint && bun run check:rust` | — |

Fixtures live in `tests/fixtures/<mode>/` (transcript, OCR, snapshot, mode, expected shape) so
no real meeting is needed to exercise the pipeline.

## Native test harness (macOS)
`tests/native/README.md` explains how to run `bluey-helper` from a terminal and feed JSON-Lines
requests (`tests/native/requests/*.jsonl`) to verify screen capture, OCR, microphone, system
audio, accessibility and observation deterministically.

## Manual QA checklist
**Authentication** — login · logout · session restore after relaunch · missing publishable key screen.
**Permissions** — grant · deny · revoke while running (audio stops, repair flow) · retry · Open System Settings links.
**Screen** — single monitor · multiple monitors · Retina scaling · fullscreen app · Spaces · mirrored/disconnected display · region capture · HUD excluded from captures.
**Audio** — microphone · system audio · headphones · Bluetooth device · device disconnect mid-session · pause/resume · levels.
**Panel** — move (⌘ arrows) · drag · resize · hide/show (⌘\) · always-on-top over fullscreen apps · remembers position per display · never off-screen · opacity/width settings · Privacy mode hides it from a screen share (Zoom/Meet/QuickTime) and the tooltip reports the state honestly.
**Shortcuts** — defaults · remap · conflict warning · disabled shortcut · registration failure message.
**AI** — fast answer · streaming · structured sections · code copy · vision (screen with little text) · failure · timeout · cancellation (new ⌘↵ while streaming) · offline banner · provider test connection.
**Modes** — every built-in mode with its fixture scenario · custom mode create/duplicate/edit/delete/set default/set active · reset built-in.
**Documents** — upload PDF/DOCX/TXT/MD · parse errors · retrieval shows in responses · delete.
**Session** — start · pause · resume · end · timeline events clickable · notes · summary · search · export · delete.
**Privacy Center** — toggles apply immediately · one-click disable all capture · data deletion counts drop to zero · reset Bluey returns to onboarding.
**Menu bar** — state text updates · every item works · quit cleans up helper and agent processes.
