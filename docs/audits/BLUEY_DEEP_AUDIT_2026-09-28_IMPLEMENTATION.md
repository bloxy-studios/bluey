# Bluey deep audit — implementation report (2026-09-28 to 2026-10-09)

Companion to [BLUEY_DEEP_AUDIT_2026-09-28.md](BLUEY_DEEP_AUDIT_2026-09-28.md). That document is the
diagnosis. This one records what was changed, how it was verified, and what is still open.

| | |
|---|---|
| Starting revision | `main` @ `1a117a5c92be75a78b429486652937d966fa3fe1` (== `origin/main`) |
| Audit commit (first on the branch) | `abc0693 docs(audit): add Bluey deep product and architecture audit` |
| Integration branch | `audit/bluey-deep-refinement-20260928` |
| Head at the end of this cycle | `d1d5f07` (all lanes merged), merged into `main` as `e851017` by PR #46; this report and the register statuses follow in a docs-only PR |
| Scale | 258 commits on top of `main`; 414 files changed, +45,096 / −4,476 lines. Of the 247 findings: 128 implemented, 67 implemented but needing real-device verification, 12 partially implemented, 40 deferred. |

## 1. How the work was coordinated

1. **Diagnosis first.**
   - 18 read-only auditors traced the product end to end.
   - A skeptical verifier re-checked every finding.
   - The adversarial pass is described in the audit. It tried to refute each Blocker/Critical finding, plus the High findings in the early groups.
   - Result: 247 distinct issues, none of them refuted.
   - The report was committed before any code changed.
2. **Twelve implementation workstreams, grouped by file ownership (audit §8.2).**
   - Wave 1: A credentials, C→B1 context then prompts, D1 audio, E1 live loop, R research, B2 modes.
   - Wave 2: D2 platform, E2 HUD, F providers/onboarding, G data/CI.
   - Each ran in its own persistent git worktree (`../bluey-wt/<ws>`, branch `audit/ws-<ws>`).
   - Shared contracts were agreed up front: `detected_question`, `ContextItem.at`, `AiRequest.scope/background`, `Present/Absent/Locked` secret states.
   - Agents committed small coherent changes, each with the finding IDs and a test.
   - The credentials workstream also had an independent security review, and its material findings were fixed before merge.
3. **Integration on this branch with `--no-ff` merges.** After each merge, the cross-component gates ran on the merged tree. Merging surfaced seams that single-lane tests could not see, and each was fixed in its own commit:
   - a duplicated `researchNote` field;
   - a migration-number collision (`0006_shortcut_defaults`);
   - stale test fixtures across lanes (`AppPaths.screenshots_dir`, the `retrieve` arity, the modes harness touching the real Keychain);
   - a real UX regression: New Chat lost its ⌘R hint when New Chat became HUD-local;
   - a mock that did not recompute protection status;
   - the built-in-modes fixture and goldens after B1's mode-text change;
   - a permissions test that did not seed the live-refresh backend.
4. **A fresh six-reviewer panel then challenged the whole diff.** Slices: security, Rust runtime, engine, UI, native, consistency/docs. Critical/High issues were adversarially verified, then fixed in five follow-up lanes (FX1–FX5) and a final docs/cleanup pass. See §6.
5. **Machine and deployment constraints shaped the process, and are recorded so the numbers can be read correctly.**
   - Only one model served everything, at 8k output tokens/min shared across all agents.
   - The machine has 4 threads and 8 GB of RAM.
   - Handling: a single shared Rust target dir behind a machine-wide build lock (source-touch on worktree switch), checkpointed agents, and at most about 5 concurrent agents.

## 2. Credential and Keychain fix (the owner's #1 complaint)

**Root cause (proven on this Mac, audit §4.2).**
- Every published build was ad-hoc signed, so every update and every `tauri dev` rebuild is a new code identity to securityd. The first data read of each Bluey Keychain item then asks for the login password.
- Bluey made this far worse. It decrypted on `has()`, on every settings save and on every AI request. It rewrote tokens on every subscription request and every boot. Deletes decrypted first. A cancelled prompt read as "no key".

**What changed (workstream A, ADR 0011).**
- A native `SecretBackend` on security-framework replaces `keyring`. It uses the same service and the same items, so saved keys keep working.
- **Attribute-only** existence checks, enumeration and deletes. These never prompt.
- Deletes check their OSStatus.
- Replace = attribute-only delete + add. The current build re-owns the item, and nothing is ever modified in place.
- An in-process cache of zeroized values and presence. Each key is decrypted **at most once per process**. Errors are never cached.
- Boot presence comes from **one** attribute-only enumeration, with **zero** decrypting reads. Settings load and save never touch the Keychain.
- Tokens are persisted **only after a real refresh**. Clerk restore no longer rewrites.
- `-25293`/`-128`/`-25308` map to distinct error codes and copy: *locked / needs approval* instead of *no key*.
- Tri-state key fields: Saved / Locked with **Allow access** / Not set. A confirmed **Remove key** action.
- **Saved credentials** in Settings → Privacy: category and label only, never values. State comes from a non-interactive probe.
- Imported **Claude/ChatGPT sessions are never refreshed**, because their refresh tokens rotate and refreshing would sign the original app out. On expiry they ask for a browser sign-in or a re-import. Antigravity/Google keep refreshing, because they don't rotate. Accounts with no recorded origin fail closed.
- Foreign import denials are reported as denials.
- Debug builds use `com.codewithabdul.bluey.dev`, so dev never steals installed-app items.
- An opt-in `BLUEY_DEV_SIGNING_IDENTITY` cargo runner signs dev builds with a stable Apple Development identity. `BLUEY_LOCAL_SIGNING_IDENTITY` does the same for local release builds.

**Acceptance questions (the brief's list).**

| Question | Answer |
|---|---|
| Who owns each secret, and where is it stored? | Bluey owns `provider:<id>:api_key`, `research:*`, `agent:anthropic:api_key`, `auth:clerk:oauth_tokens` and `account:<id>:oauth_tokens`. They live in the login keychain, service `com.codewithabdul.bluey` (`.dev` for debug builds). Foreign: Claude Code's `Claude Code-credentials` and Antigravity's `gemini`/`antigravity` items; Codex uses `~/.codex/auth.json`. Inventory: audit §4.3(a). |
| Which executable reads it, and when? | Only the Rust app process. The helper and the research sidecar never touch the Keychain; the sidecar gets per-job env. Each Bluey item is decrypted at most once per process, on first use. Foreign items are read **only on an explicit Import click**. |
| Bluey-owned or foreign? Is a dialog expected, and can it recur? | Bluey-owned on a stably signed build: no dialog. After an **ad-hoc** update: at most one dialog per item actually used, not per request. With "Always Allow", none after that. Foreign import: one legitimate dialog per Import until "Always Allow". |
| Does debug/release signing affect access? | Yes. Ad-hoc and unsigned identities are per build; Apple Development and Developer ID are stable (measured, §4.2). Self-signed certificates do not help. Dev builds can opt into stable signing. Distributed builds need **Developer ID (owner action)**. |
| How are imported credentials handled after the first import? | They are copied into Bluey's own item and never re-read from the foreign store. Rotating imports are never refreshed; on expiry the user re-imports (one foreign dialog) or signs in with the browser. |
| Do provider operations ever ask the WebView for a secret? | No. Keys go Keychain → Rust cache → provider headers. The WebView allow-list (`set`/`has`/`delete` + state) is byte-identical to baseline, and the new health commands return states and labels only. |
| Do values ever leave native memory unnecessarily? | No. Values are `Zeroizing` in Rust and never cross IPC, and are never logged or written to SQLite. Sidecars get only the keys a research job needs, as env. |

**Verified on this Mac.**
- The non-interactive ACL probe (§4.2).
- The opt-in real-Keychain backend test: a round trip with user interaction disabled, **passing** on the integrated branch.
- The dev signing runner, exercised on real binaries:
  - `auto` signs the `bluey` binary with the Apple Development identity under the identifier `com.codewithabdul.bluey.dev`, with a certificate-based designated requirement;
  - other binaries and the unset case pass through unsigned.

**Still owner work.** Developer ID signing and notarization for Latest and Nightly. Until that ships, each ad-hoc update still costs one approval per used item (never per request), and macOS permissions reset. The post-update repair card (MAC-001) now explains this and links each System Settings pane.

## 3. Prompt changes (workstream B1)

The full before/after is in `docs/audits/2026-09-28/prompt-eval-before-after.md`.
- The static layers shrank from **653 to 529 tokens**, and every evaluated case composes fewer tokens than before.
- **Voice model:** speak-as-user, write-as-user or explain-to-user, per request, instead of a global "write as me".
- **One precedence line:** safety > custom mode > contract > built-in guidance > style.
- **Trusted versus untrusted:** the typed question renders last as "My question:". Personal instructions move to the system prompt. All screen, transcript, document and web text is fenced in per-request **nonce** blocks, with forged structure defanged.
- **Optimizer:** no longer deletes answers.
- **Coding:** asks for the solution once, first, with no duplicate `code` field.
- **Schemas:** JSON-schema keys go content-first; Anthropic numeric constraints are stripped, with a sticky fallback.
- **New shapes:** debug and uncertainty.
- **Vision gate:** measures sufficiency on OCR only and honours visual cues.
- **Evaluation:** `tests/prompt-eval/`.
  - A deterministic invariant matrix over the audit's evaluation cases.
  - Goldens for six representative prompts.
  - An opt-in model-graded tier (`BLUEY_PROMPT_EVAL_LIVE=1`, not run here because it spends the owner's API credits).

## 4. Other major changes by area

The changes below are grouped by product area; credentials and prompts are in §2 and §3, and the review's credential fixes (FX3) are in §6. Fixes from the final review lanes (FX1–FX5) and the closing cleanup (FX7) are folded in where they apply.

### Context engine

- A heard question with no typed instruction becomes one `detected_question` item ("<speaker>: <text>") that the budget never drops (CTX-005). Transcript items carry their start time (`ContextItem.at`) and render oldest first (CTX-006).
- Earlier chat turns are conversation memory on every ask, with or without a session: the last two as Q/A (the newest keeps a capped code block), older ones as short summaries under "Earlier in this chat" (CTX-007).
- A failed screen capture no longer fails the snapshot. The accessibility tree, transcript and app identity are kept, and a `screen_unavailable` warning reaches the HUD with an Open Settings action (CTX-010, UX-002). With the HUD screen toggle off, no screen image, OCR or accessibility text is captured, even on ⌘↵ (UX-002).
- Screen context is leaner. It adds one ~20-token active-app item (CTX-012), removes accessibility lines already in the OCR and skips that item when less than 20% of it is new (PERF-005), and compresses a long typed paste as head plus tail so a question at the end survives (CTX-017).
- A typed ask that shares no keyword with the screen drops the OCR, the window text and all but the last two transcript turns (PERF-006). FX1 narrowed this floor after the review: task verbs (summarize, reply, explain and others), conversation cues and transcript overlap now skip it, and when it applies it keeps the OCR headline and the earlier-conversation summary (relevance-floor-drops-needed-context).
- Snapshot latency: session reads run alongside capture and load only the newest events (PERF-012). OCR races a 150 ms soft deadline; when it is slower, the snapshot goes out with the image and an `ocr_pending` warning (PERF-002). Display captures reuse the shareable-content list for up to 1.5 s while the display layout is unchanged (PERF-015).
- A capture with no display id targets the display holding the frontmost window, then the one under the mouse, then the main display (CTX-013). "Selected region" is gone from Screen settings because no region picker existed; a stored `region` now reads as active window (FEATURE-006).
- Semantic document matches below a raw cosine of 0.25 are dropped, and only chunks embedded with the current model are compared. The 0.25 value is a heuristic still to check against real embeddings (CTX-015). FX1 made the mock's `documents_retrieve` match Rust keyword retrieval (mock-retrieve-more-permissive).
- Intent and vision-gate changes (CTX-001, CTX-009, CTX-011) are in §3. FX1 narrowed the error-screen marker to error-report lines, so code that only names `ValueError` or `IOException` no longer gets the debug shape (debug-shape-on-ordinary-code).

- FX5 limits the focus rule to screen captures: a request without a display id means the main display again for system audio and the observer, and display and region captures ask for the focused display explicitly (null-display-semantics-leak).

Partial: CTX-001 (recency limit on the fusion-side fallback question is unchanged).

### Modes

- Files attached to a mode reach the prompt in every mode: each ask searches the active mode's own documents with no kind filter (CTX-002). Every mode also runs a small keyword pass over session and global documents, and personal-instruction documents load on every ask (CTX-003).
- Candidate modes pin the résumé's leading chunks (about 800 tokens) when the question matches none of it or is a standard intro or behavioral question. Other modes never pin it (CTX-008).
- A mode's files live in one place, its mode-scoped documents, and Duplicate copies each file with its chunks (DEBT-010). Deleting a custom mode deletes its documents, chunks and search rows in one transaction, and migration 0005 removes files that earlier deletes left behind (DATA-005).
- Choosing a default mode switches to it at once when no session is running, and launch opens the default mode, or General if it no longer exists (MODE-003). FX2 removed the resume-a-session branch, which DATA-002 had made dead and which left a relaunch after a crash in the stale mode (mode-resume-branch-dead).
- Deleting the default mode resets the default to General. Deleting the active mode switches to the default, or to General if the default is gone too (MODE-013).
- Unedited built-in modes refresh to the shipped text at startup, tracked by a stored fingerprint. Edited ones keep their text, and Reset to default restores the shipped text (MODE-005).
- Mode fields are validated in Rust and in the editor (name 60, description 300, instructions 4000 characters, kebab-case icon, allowed model role), with errors shown inline (MODE-010). An explicit null clears a mode's group or preferred role, and a missing field keeps it (MODE-009). Reset cancels pending saves first, and failed duplicate, set-default, reset and create actions show an error (UX-019).
- Team Meeting (decisions, action items, important statements, topic changes) and Lecture (important statements, topic changes) record detections that need no answer as session events for the timeline and summary. Spoken text is stored only when transcripts are kept (MODE-006).
- Requests carry the mode's preferred model role, and Rust routes by it first. A mode change clears prepared answers; a suggestion already in flight finishes on its original role (MODE-012).
- Built-in mode definitions are generated from Rust into one JSON fixture that the mock and the tests load, with a drift test (TEST-014). Shape and voice changes (MODE-001, MODE-002, MODE-004) are in §3.

Partial: TEST-014 (fixtures for default settings and shortcut bindings not generated yet).

### Live listening and audio

- The transcript ring tags each entry with its listening run and session (CTX-004). FX2 widened the snapshot scope to the whole session when one is active, so restarting or rerouting listening inside a session keeps what was said earlier (ring-run-scope-drops-context).
- With Privacy → Cloud AI off, listening uses on-device Apple Speech, and turning the switch off mid-run moves the run there (SEC-003). FX2 added `requireOnDevice`: in that state the helper refuses Apple's server fallback and reports `audio.speech_on_device_unavailable` (cloud-ai-off-apple-server-speech).
- Start is single-flight, and only Rust reacts to the listening shortcut, so a duplicate toggle gets the current status back and cannot tear down the start that won (LIVE-007). After a start timeout a best-effort stop goes to the helper, and `audio_already_running` triggers a stop-then-start resync (MAC-009).
- Gemini Live and Voice Live share one reconnect policy: backoff from 0.5 s doubling to 8 s, 5 s send timeouts and a 15 s read watchdog (LIVE-004). Each outage raises one `audio.stt_degraded` notice that clears when transcripts flow again; a source that gave up reopens after 10 s, and configuration errors move listening to Apple Speech (LIVE-013).
- When the helper exits, a live run stops and is re-issued once on the replacement helper, and an info toast replaces the "native helper is not running" error (MAC-004, UX-041). FX2 re-issues Smart observation the same way (observation-not-resumed-after-helper-restart).
- Apple Speech partials and finals carry an `utteranceId` (UX-010). A recognizer reset after a pause commits the last partial as the final (MAC-002). Each recognition request is a generation, so a retired request cannot restart the live one; this ends the request churn after the first 55 s rotation (MAC-003).
- `audio.started` reports the speech locale and whether recognition runs on the device, and a one-time `audio.speech_server` notice appears when a locale has no on-device model (MAC-007). The expected Apple Speech fallback is one info notice per app run, not an error on every start (UX-023).
- A failed microphone rebuild after a device change retries on the next change, and a vanished device falls back to the default input (MAC-005). The speech capability is probed once on a background queue at startup instead of on the helper's read queue (MAC-011).
- A run that joins a session with a transcript offsets its segments onto that session's timeline (DATA-008). Apple Speech requests ask for punctuation, so finals carry "?" for question detection (CTX-014).
- If rerouting to Apple Speech fails, FX2 now stops the run and ends its auto-session instead of showing Listening (reroute-failure-leaves-listening).

- FX5 gives each committed final its own audio times, commits short utterances after a pause, and keeps `language: auto` on the Mac: the user's locale is used only when it has an on-device model, else en-US (committed-final-timing-is-request-start, reset-heuristic-short-utterances, auto-locale-leaves-device).

Partial: reroute-failure-leaves-listening (fixed without a regression test).

### Live suggestions and the ask loop

- Every request carries a `scope` (ask, live, prepare or classify) and a `background` flag, and Rust supersedes an older request only when session and scope match (LIVE-002).
- Background and non-answer requests never drive the app state machine, and a cancel returns it to idle when no other answer is running (LIVE-003). The review found that background snapshots still left the app in Analyzing after every detected question; FX2 makes them skip that phase (background-snapshot-stuck-analyzing).
- The state machine recovers from Error: a new capture or answer clears it, and mic start and stop register in every state except Booting and AuthRequired (UX-003).
- Live suggestions stop like asks: Esc, Stop, a manual ask, New chat and ⌘⇧↵ cancel them, and a stopped suggestion is not saved (LIVE-001).
- `shouldSurface` decides which heard questions get an answer. It filters back-channel and meta phrases, drops near-duplicates within 90 s, applies an 8 s same-speaker cooldown that a correction ("sorry, I mean") bypasses, drops queued questions older than 20 s, and raises its bar a little after each dismissal (LIVE-009). An unfinished fragment waits about 900 ms to merge with the same voice's next final (LIVE-010).
- FX1 closed two gaps in that gate: a question deferred behind a hidden HUD now gets the same 20 s staleness check (deferred-question-bypasses-staleness), and taking the prepared answer or starting a new chat no longer counts as a dismissal (take-prepared-counts-as-dismissal).
- While the HUD is hidden nothing is generated; only the newest question is kept, and it is prepared if the HUD returns within the TTL (LIVE-012). Prepared answers and the ⌘⇧↵ hint expire (LIVE-016), Stop listening clears held, queued and deferred questions (LIVE-017), and suggestions see the thread's earlier answers (LIVE-014).
- ⌘⇧↵ over a streaming turn stops it, then shows the prepared answer and saves it (LIVE-011, DATA-007). A length retry keeps the visible draft until the retry's text is longer (LIVE-015).
- Retry and Regenerate re-send the turn's own request (UX-011). FX1 keeps a regenerated suggestion in the spoken voice (regenerate-suggestion-loses-spoken-voice), stops Retry and Regenerate from sending heard speech as a trusted typed question (heard-question-promoted-to-trusted), and documents that Regenerate reads the screen again (regenerate-recaptures-screen-doc-mismatch).
- `tests/ui/live-races.test.tsx` covers 13 races, among them Esc/Stop, manual ask, ⌘⇧↵, New chat, correction, Retry/Regenerate, hidden HUD, Stop listening and mode switch (TEST-003).

### Research

- Cancel publishes `failed{cancelled}` at once, aborts in-flight Exa and Firecrawl fetches, and kills the sidecar if the job still exists after a 2 s grace (LIVE-005). `jobs_tests.rs` drives the job table against a scripted fake sidecar (TEST-005).
- Quick search runs under a 10 s deadline with parallel scrapes capped at 6 s each, and keeps snippets when pages are slow. The HUD shows "Searching the web…" with a working Skip (LIVE-006), and progress reads as plain copy such as "Found N sources" (UX-030).
- Results enter the prompt as separate budget items: the snippets, each scraped page (capped at 6k characters, with a source header) and the agent report (capped at 12k). Citations keep only what the budget kept (AI-002).
- Deep research gets a deadline of the ask timeout minus 15 s. When turns or time run out it writes a "Research stopped early" report from the sources gathered, and it fails only with no evidence (AI-008). Cancel reaches the deep agent and the search paths (AI-010).
- The sidecar returns validated model citations plus URLs linked in the report, de-links URLs the tools never returned, and gives answer citations their own ids (AI-009). The agent prompt asks for a Sources section of sources actually used, adds the date and tool-turn budget, and marks tool results as untrusted (AI-013).
- Web queries strip identity only: emails, phone numbers, handles and the signed-in user's names (SEC-013). FX1 removed the stripping of capitalized words found in résumé chunks, which had dropped topic words such as "Machine", "Learning" or "Kubernetes" from searches (research-query-overstrip).
- Research availability reflects what can run: deep research needs an Exa key, Firecrawl is requested only when scraping is available (PROV-013), and the sidecar reports its variant and backends so lite builds do not offer Claude (PROV-009). FX2 limits that probe to the Claude backend, so Gemini asks no longer start the sidecar to wait for it (agent-info-probe-on-ask-path).
- With Cloud AI off, FX2 refuses Exa search, Firecrawl scrape and agent starts with `privacy.cloud_ai_disabled` (cloud-ai-switch-not-enforced-for-research). The Claude CLI runs with telemetry and error reporting off and without the Exa and Firecrawl keys (SEC-017).
- A failed research step leaves a short note that the HUD shows under the answer (AI-016, UX-035). Links open only for https, http and mailto, and a refusal shows a toast (UX-006).
- ADR 0004 and `docs/AGENT_SIDECAR_PROTOCOL.md` document `agent.info`, deadlines, cancellation, forced reports, flush and citation validation (DOC-007). AI-014 made the sidecar wait for queued output before exiting; FX5 makes it wait for the last frame's own write, which also holds in the compiled Bun sidecar (flushstream-ineffective-in-bun).

- FX5 keeps parenthesised source URLs and code spans intact in the citation check (citation-sanitizer-breaks-paren-urls), and a report forced by the deadline now carries the turns and tokens already spent (deadline-evidence-drops-usage).

Partial: AI-016 (the research note is not persisted with the saved answer); agent-info-probe-on-ask-path (TTL cache or startup warm-up for the Claude probe deferred).

### Providers, routing and onboarding

- When a role's assignment points at an unusable account or a missing provider, the router falls back to the first usable API-key provider (Gemini preferred), and the answer records the fallback reason (PROV-001). With nothing usable, the error names the provider and the cause (missing or locked key, disabled, account state) and offers Open AI settings (UX-007).
- Reasoning-model families never get temperature or other sampling knobs on Azure, OpenAI-compatible or Anthropic API-key requests (PROV-002). Reasoning level and latency map to `reasoning_effort` or Anthropic thinking where the family supports it (PROV-005).
- Provider errors are read from a bounded body that is never logged. A missing deployment or model is named, 429 honors retry-after, and 429/5xx are retried (PROV-004). A 401 on an unexpired account token forces one single-flight refresh before falling back to reauth; rotating imports are never force-refreshed (PROV-007).
- When a provider rejects structured output, the request is resent with the schema in the prompt, and that choice sticks per provider and model for the process (PROV-008, PROV-003). FX2 narrowed the trigger to 400s that say `response_format` is unsupported, so other 400s, such as a bad schema, are no longer retried silently (response-format-rejection-too-broad).
- Audio-file transcription falls back to the first enabled Gemini provider with a key when the assigned provider cannot batch-transcribe, and asks for one otherwise (PROV-006).
- `ai_readiness` runs the router against real provider state. Onboarding's Test AI and Ready steps and a readiness banner in the AI tab use it, and never claim ready otherwise (ONB-001). FX4 made Test AI work for subscription-account providers, testing exactly the provider and model the router picked (onboarding-test-ai-ignores-account-providers).
- A provider's first key fills the unassigned roles with its recommended models and shows a toast with Undo (FEATURE-004); FX4 made that Undo clickable (first-key-undo-unreachable). Remove provider asks for confirmation, lists the roles the provider serves and deletes its key (UX-008).
- Settings that fail to decode fall back one section, then one field, at a time. An unknown provider kind is dropped on its own, and the raw stored text is kept in `settings.backup` (DATA-009).
- Onboarding resumes at the saved step (ONB-004). Output languages are stored as codes with names as labels (UX-042), and About links open the project's GitHub readme and issues (UX-043). FX4 labels rate-limited accounts "(rate limited)" instead of "(not connected)" (rate-limited-account-labelled-not-connected).

Partial: FEATURE-004 (key-prefix inference, transcription-provider choice and collapsing Models behind Advanced not built).

### HUD and UX

- One persistent composer keeps its draft across layout changes and live turns, and takes focus only when the panel appears (LIVE-008). FX4 deleted the unused two-layout wrappers, and the keyboard tests now mount the composer that production renders (dead-composer-wrappers).
- Streaming updates the store at most once per animation frame, and finished turns do not re-render or re-parse while a later answer streams (PERF-003).
- HUD errors appear in one inline notice row inside the measured frame instead of a toast over the toolbar (UX-013). FX4 lets error notices wrap to two lines so the recovery text stays readable (hud-notice-truncates-recovery-text). Recovery actions run through one runner that reports their own failures (UX-036).
- Esc clears a non-empty draft before it hides or cancels anything, and clearing the thread shows "Chat cleared · Undo" for 5 s (UX-012).
- The Screen toggle state lasts for the session, and the notice row says when the screen is not included (UX-002). FX4 gave the Screen and Content-protection toggles stable labels, with the state in `aria-pressed` (toolbar-toggle-label-and-pressed).
- Panel opacity changes only the background, so text stays opaque, and HUD-specific muted tokens keep contrast (UX-014). Code blocks and Mermaid diagrams follow the light or dark theme (UX-005).
- Shortcut hints come from the saved settings and hide when a shortcut is disabled, and the suggestion pill is a real button (UX-026). FX4 made plain ⌘R in the focused composer start a new chat and kept the ⌘↵ and ⌘⇧↵ keycaps visible (hud-local-cmd-r-dead-in-composer).
- Each window, chat turn and Mermaid diagram has its own error boundary, so one bad render no longer blanks a window (UX-038). A polite live region announces thinking, answer ready, listening and errors once per change (UX-025).
- A new turn scrolls its top into view, and following continues only while its first line is visible (UX-027). Under each answer, a muted line shows the model, latency and any fallback reason (UX-035).
- The eye button reads and writes the saved display mode, so it is correct after a relaunch or a tray toggle (UX-004). Platform covers the protection status it shows.

Partial: UX-012 (answers asked outside a session are not listed in History).

### Platform

- Default global shortcuts no longer take editing or app chords. New Chat (⌘R) and Settings (⌘,) are HUD-local, Scroll is ⌘⌥↑/↓, and a one-time migration moves only rows still on an old default (UX-001). FX2 then moved window Move from ⌃⌥ + arrows, which Rectangle and Magnet use, to ⌃⌥⌘ + arrows, rewrote the unshipped migration 0006 to match, and added those window-manager chords to the known system shortcuts (new-move-chords-collide-with-window-managers).
- A binding that macOS refuses shows "Not active" with the reason in Keybinds and in onboarding (UX-039). Failures to apply shortcuts, content protection, launch at login, Smart observation or the retention sweep show an error that opens the matching settings tab; an observation-status indicator in Screen settings is not built (UX-037).
- Bluey runs as an accessory app with no Dock icon or ⌘-Tab entry, and keeps the Edit key equivalents for Settings fields (MAC-010). The HUD opens at its saved pinned or always-on-top level (MAC-016). Opening Bluey.app again from Finder or Spotlight shows onboarding until it is complete, then the HUD (UX-024).
- After an update, permissions lost with the code identity are detected per app version and listed on a repair card in Settings → Permissions, each with an Open System Settings action (MAC-001). FX4 reworded the card so it no longer claims the running build is unsigned (update-repair-card-unconditional-unsigned-claim). Developer ID signing is still owner work (§2).
- Permission status in onboarding and Settings refreshes on mount, on window focus and every 2 s while visible. A restart offer and mapping "denied" to "not requested yet" are not done (ONB-002).
- Content protection reports a partial state on macOS 15+ or an unknown version, and the copy says ScreenCaptureKit sharing may still show Bluey (SEC-004). At boot, Privacy mode protects every window before the HUD first shows, and the tray toggle patches the saved display mode (SEC-012).
- FX4 made the HUD eye and the Privacy tab read the actual protection status (`useCaptureProtection`), so they no longer overclaim after SEC-004 (privacy-eye-overclaims-after-sec004). The Privacy tab only patches settings and leaves protection to the Rust side effect (privacy-tab-double-protection-lane).
- A bootstrap failure (database open, migration, settings) shows one native dialog with the error, the log folder, Reveal Data Folder and Quit, instead of the app vanishing (CRIT-003).
- Relaunching for an update shuts down audio, the agent sidecar and the helper first, so none is orphaned (MAC-014).

### Data, sessions and privacy

- An interrupted session is no longer restored at launch. It is marked completed at its last activity with a `session_recovered` event, or deleted if history is off (DATA-002).
- Answers asked outside a session are removed by Delete all, by ending a session with history off and by retention, and are not saved while history is off (DATA-003). Deleting the live session ends it in the HUD, and Delete is disabled for a session that has not ended (DATA-006).
- SQLite `secure_delete` and FTS5 secure-delete are on, and every deletion or retention pass ends with `wal_checkpoint(TRUNCATE)`, plus VACUUM on bulk resets (DATA-010).
- The pre-migration database backup added with the boot-failure work (CRIT-003) kept deleted text. FX2 keeps only the current version's backup at open, and retention, session and response deletes, audio clear, document deletes and resets drop it after the WAL checkpoint (db-backups-never-pruned, db-backup-defeats-deletion).
- Screen frames have a lifecycle: evicted and inlined frames are deleted, kept screenshots are copied into the data folder, Delete screenshots and Reset empty both folders, and the helper deletes frames older than 10 minutes (DATA-001). FX2 moved the kept-screenshot write off the ⌘↵ path (snapshot-copy-on-hot-path).
- A pdf-extract panic returns a parse error instead of aborting the app, and DOCX entries are read with a decompressed-size cap (CRIT-002). The panic hook is described under Reliability.
- Logs are redacted on the formatted line, with patterns for escaped JSON, access, refresh and id tokens, client secrets, passwords and bare JWTs (SEC-016). Log files older than 14 days are deleted at startup, and Reset all data deletes every log file (DEBT-009).
- Session detail shows answers, improvements and mode sections, and the Markdown export includes them (UX-016). Long transcripts are summarized from the start and the end, and the summary says which minutes were left out (AI-007). Sessions load in pages of 50 with Load more (UX-017).
- With Privacy → Cloud AI off, AI requests, document embedding (including boot re-embedding) and audio-file transcription are refused with `privacy.cloud_ai_disabled` (SEC-003). E1 recorded SEC-003 as partial only because the live-audio half belonged to D1, which implemented it; FX2 closed the speech and research gaps (see the audio and research areas).
- Privacy settings show raw audio as "Never kept", since nothing records audio to disk (FEATURE-003). Sign-out stops audio capture and AI work (SEC-010); the credential side of sign-out is in §2.

### Reliability (panic handling)

- Release builds use `panic=unwind` so a parser panic can be caught, and a `compile_error!` blocks switching the profile back to abort (CRIT-002).
- `bluey_core::panic::contain` wraps `catch_unwind` and marks the code inside as contained. The PDF parser runs inside it, so its panics come back as parse errors (CRIT-002, panic-unwind-no-hook-silent-task-death).
- FX3 installs a panic hook at bootstrap. It logs the location and thread, never the payload, calls the previous hook, and aborts unless the panic is contained, so a panic in a Tauri command or a spawned task no longer dies silently and leaves the app half-updated (panic-unwind-no-hook-silent-task-death, panic-unwind-no-hook).
- A child-process test checks that an uncontained panic aborts and logs its location without the payload, and `bluey-core` tests pin what counts as contained.
- Comments that still described `panic = "abort"` and the old build-failure path now match the code (panic-unwind-no-hook, stale-docs-router-and-panic).
- Related containment: a boot failure shows a dialog instead of the app vanishing (CRIT-003, Platform), and a render error stays inside one window, turn or diagram (UX-038, HUD and UX).

### CI and release

- The updater signing key and password are set only on the release step, and `release.sh` strips them from installs, type checks, lint, tests and sidecar builds (SEC-008). FX3 split the build: the app compiles with `tauri build --no-bundle` without the key, and only `tauri bundle` receives it (updater-key-still-in-full-tauri-build).
- Nightly skips a commit unless its latest `ci.yml` push run on main succeeded, even when forced (TEST-006).
- The macOS CI job runs `scripts/test-helper.sh` after building the helper (TEST-008).
- Sidecar builds write a stamp with a hash of their sources, and `ensure-sidecars.sh` rebuilds any sidecar that is missing, unstamped or stale (TEST-009).
- Vitest runs separate unit and UI projects with a 15 s UI timeout. The session-rename tests paste text and wait on roles instead of typing key by key, and release-script test timeouts rose to 60 s after timing out at load average 68 (TEST-022).
- Unit tests cannot reach the login keychain: `SecretsStore::new()` panics under `cfg(test)`, and the sessions harness uses the counting fake (sessions-tests-hit-real-keychain).

### Settings and cleanup (final pass, FX7)

- A second press of the toggle-panel shortcut while the HUD is visible focuses the HUD composer; the other app stays active (UX-015).
- Smart observation is shown as not yet available and starts nothing, and the interval select offers the real default (FEATURE-002, UX-031). The Follow active display toggle, which nothing honoured, is hidden (UX-032).
- Reset all data hides the HUD and returns to onboarding (DOC-003). Finishing onboarding stays in the wizard with an error when the completion cannot be saved (ONB-005).
- The microphone picker offers System default again (UX-033). Copy answer copies the title, content and sections as plain text, and a failed rating says so (UX-028).
- The Research role says what it drives and points to the Deep research backend setting (PROV-011). The mode editor offers only context chips that change what a mode gathers (MODE-011).
- Dead paths are gone: the Clear AI cache action and its always-empty table path (DEBT-011), the unused `bluey_core::budget` module (DEBT-012) and the `privacy.debugLogTranscripts` switch nothing read, which old settings still load without (DOC-008). A ContextTab comment is corrected (DOC-006).

## 5. Tests and gates

| Gate | Baseline `1a117a5` | End of cycle |
|---|---|---|
| TypeScript (app + agent sidecar) | pass | pass on the final merge `d1d5f07` |
| ESLint, 0 warnings | pass | pass on the integrated tree (`f324e14`), in each lane, and in CI on PR #46 |
| Vitest | 645/651, 6 load timeouts | 1,214 passed, 26 skipped, 1 failed in FX7's full run (121 files). The failure is the load-sensitive `tests/ui/windows.test.tsx` › SettingsWindow General page test, which also fails before this cycle's changes. CI's Frontend job passed on PR #46. |
| Rust host crates | 417 tests, fmt, clippy | bluey-core 127, bluey-protocols 217, bluey-storage 96, plus oauth and fingerprints; fmt and clippy `-D warnings` clean. CI's Rust job passed on PR #46. |
| App crate (macOS) | 63 tests | 192 passed and 1 ignored on the integrated tree (`f324e14`); clippy `-D warnings` clean in the FX2 and FX7 lanes. The macOS CI job runs it on the merged tree. |
| Swift helper XCTest | 40 | 70 of 70 on the final merge |
| Release scripts | pass in isolation | pass; FX3 added release-script tests for the bundling split |
| Prompt eval | none | `tests/prompt-eval/`: invariant matrix and 6 goldens in Vitest; the live tier is opt-in |

How the gates ran:
- **Lanes.** Every lane ran its own gates before it was merged.
- **Merges.** After each merge the cross-component gates ran on the merged tree, and each failure was fixed in its own commit (§1).
- **Final merge.** The closing merges (FX6 docs, FX7 cleanup, FX5 native) were checked locally with the TypeScript checks and the Swift suite. Rust tests and the full Vitest run on that commit were left to CI: while the cycle closed, other work on this machine held the load average between 300 and 500, and a cold app-crate build took about 30 minutes.
- **CI on PR #46.** The release-script, Frontend and Rust jobs passed; the macOS job was still running at merge time. Nightly publishes only when CI on `main` is green.

## 6. Final review and follow-up fixes

**How the review ran.**
- Six fresh reviewers each owned one slice of the full diff from the audit commit (`abc0693`) to the integrated head: security, Rust runtime, engine, UI, native (Swift helper, research sidecar, stamp scripts) and consistency (merge commits, the six integration-fix commits, cross-workstream overlaps, a doc sweep and 10 sampled Critical/High audit findings).
- Each reviewer returned its issues with evidence and a proposed fix, plus a list of what it checked and found sound.
- They raised 52 issues, none of them Critical:

| Slice | Issues | High | Medium | Low |
|---|---|---|---|---|
| Security | 8 | 0 | 2 | 6 |
| Rust runtime | 13 | 2 | 2 | 9 |
| Engine | 7 | 2 | 2 | 3 |
| UI | 10 | 0 | 2 | 8 |
| Native | 8 | 0 | 2 | 6 |
| Consistency | 6 | 1 | 3 | 2 |
| **Total** | **52** | **5** | **13** | **34** |

- Two pairs describe one defect seen from two slices: the missing panic hook (security and Rust) and the pre-migration database backup (Rust and consistency).
- **Adversarial verification.** A separate agent tried to refute each of the five High issues against the code, and none was refuted. Three stayed High (background-snapshot-stuck-analyzing, debug-shape-on-ordinary-code, relevance-floor-drops-needed-context). The two backup issues were re-rated Medium (db-backups-never-pruned, db-backup-defeats-deletion). Each verdict went to the fix lane with its issue, together with the verifier's smallest-fix proposal. Medium and Low issues were not separately verified.
- **Fix lanes.** 51 issues went to five lanes grouped by the code they touch: FX1 TypeScript engine (9), FX2 Rust core (14), FX3 security (9), FX4 UI (10) and FX5 native helper, sidecar and docs (9). One issue went to no lane (see below).
- **Gates.** FX2 and FX3 passed all of their gates, including the Rust test suites. FX4 passed its TypeScript gates and touched no Rust or Swift. FX1 passed except one UI test (`tests/ui/windows.test.tsx`, a `waitFor` timeout) that also failed 3 of 3 runs at `ed585cd`, before the lane's changes. FX5 passed its Swift suite (67 tests) and sidecar checks in its lane; on the integrated tree the helper suite passes 70 of 70.

**Issues and outcomes.** Statuses come from each lane's result file. Of the 51 issues in FX1–FX5, 49 are implemented and 2 are partial.

| Issue | Severity | Slice | Fix lane | Outcome |
|---|---|---|---|---|
| relevance-floor-drops-needed-context | High | Engine | FX1 | Implemented: cues and transcript overlap now skip the floor |
| debug-shape-on-ordinary-code | High | Engine | FX1 | Implemented: marker matches error-report lines only |
| research-query-overstrip | Medium | Engine | FX1 | Implemented: query strips identity only |
| regenerate-suggestion-loses-spoken-voice | Medium | Engine | FX1 | Implemented: `isSpokenAsk` keeps the spoken voice |
| heard-question-promoted-to-trusted | Low | Engine | FX1 | Implemented: fallback rebuilds a heard-question request |
| deferred-question-bypasses-staleness | Low | Engine | FX1 | Implemented: 20 s staleness check before replay |
| mock-retrieve-more-permissive | Low | Engine | FX1 | Implemented: mock matches Rust keyword retrieval |
| take-prepared-counts-as-dismissal | Low | UI | FX1 | Implemented: ⌘⇧↵ and New chat are not dismissals |
| regenerate-recaptures-screen-doc-mismatch | Low | UI | FX1 | Implemented: behavior kept, docs corrected |
| background-snapshot-stuck-analyzing | High | Rust | FX2 | Implemented: background snapshots skip Analyzing |
| db-backups-never-pruned | High (verified Medium) | Rust | FX2 | Implemented: only the current version's backup kept |
| db-backup-defeats-deletion | High (verified Medium) | Consistency | FX2 | Implemented: deletions now drop the backup |
| ring-run-scope-drops-context | Medium | Rust | FX2 | Implemented: transcript context scoped to the session |
| cloud-ai-off-apple-server-speech | Medium | Consistency | FX2 | Implemented: `requireOnDevice` refuses Apple server speech |
| reroute-failure-leaves-listening | Low | Rust | FX2 | Partial: fixed, but no regression test |
| agent-info-probe-on-ask-path | Low | Rust | FX2 | Partial: Claude-only probe; cache or warm-up deferred |
| mode-resume-branch-dead | Low | Rust | FX2 | Implemented: launch always loads the default mode |
| response-format-rejection-too-broad | Low | Rust | FX2 | Implemented: fallback only for unsupported `response_format` |
| snapshot-copy-on-hot-path | Low | Rust | FX2 | Implemented: screenshot write moved off the ⌘↵ path |
| cloud-ai-switch-not-enforced-for-research | Low | Rust | FX2 | Implemented: research refused when off; mock-parity test only |
| observation-not-resumed-after-helper-restart | Low | Rust | FX2 | Implemented: observation re-issued after helper restart |
| stale-docs-router-and-panic | Low | Rust | FX2 | Implemented: two doc comments corrected |
| new-move-chords-collide-with-window-managers | Low | Rust | FX2 | Implemented: Move is now ⌃⌥⌘ + arrows |
| signout-failed-delete-stuck-and-resurrects | Medium | Security | FX3 | Implemented: stays signed out; delete retried at launch |
| panic-unwind-no-hook-silent-task-death | Medium | Security | FX3 | Implemented: hook logs, aborts unless contained |
| panic-unwind-no-hook | Medium | Rust | FX3 | Implemented: same fix as the row above |
| probe-disables-ui-process-wide-vs-foreign-import | Low | Security | FX3 | Implemented: probe and Import share one lock |
| legacy-accounts-without-origin-stop-refreshing | Low | Security | FX3 | Implemented: neutral reconnect prompt; still fails closed |
| failed-disconnect-leaves-unremovable-item | Low | Security | FX3 | Implemented: leftover tokens deleted at next launch |
| sessions-tests-hit-real-keychain | Low | Security | FX3 | Implemented: fake backend; real store panics in tests |
| updater-key-still-in-full-tauri-build | Low | Security | FX3 | Implemented: key reaches only `tauri bundle` |
| stale-docs-shared-session-refresh | Low | Security | FX3 | Implemented: three stale texts corrected |
| hud-local-cmd-r-dead-in-composer | Medium | Consistency | FX4 | Implemented: ⌘R works from the focused composer |
| privacy-eye-overclaims-after-sec004 | Medium | Consistency | FX4 | Implemented: eye and Privacy tab read real status |
| first-key-undo-unreachable | Medium | UI | FX4 | Implemented: info-toast actions are clickable |
| onboarding-test-ai-ignores-account-providers | Medium | UI | FX4 | Implemented: Test AI follows router readiness |
| privacy-tab-double-protection-lane | Low | UI | FX4 | Implemented: tab only patches settings |
| hud-notice-truncates-recovery-text | Low | UI | FX4 | Implemented: error notices wrap to two lines |
| toolbar-toggle-label-and-pressed | Low | UI | FX4 | Implemented: stable labels, state in `aria-pressed` |
| update-repair-card-unconditional-unsigned-claim | Low | UI | FX4 | Implemented: copy no longer claims an unsigned build |
| dead-composer-wrappers | Low | UI | FX4 | Implemented: unused wrappers deleted |
| rate-limited-account-labelled-not-connected | Low | UI | FX4 | Implemented: label reads "(rate limited)" |
| citation-sanitizer-breaks-paren-urls | Medium | Native | FX5 | Implemented: parenthesised URLs and code spans kept |
| committed-final-timing-is-request-start | Medium | Native | FX5 | Implemented: finals span their own audio times |
| flushstream-ineffective-in-bun | Low | Native | FX5 | Implemented: waits for the last frame's own write |
| null-display-semantics-leak | Low | Native | FX5 | Implemented: only captures follow focus |
| reset-heuristic-short-utterances | Low | Native | FX5 | Implemented: short utterances commit after a pause |
| auto-locale-leaves-device | Low | Native | FX5 | Implemented: `auto` stays on the Mac or uses en-US |
| deadline-evidence-drops-usage | Low | Native | FX5 | Implemented: forced reports keep turns and tokens |
| helper-protocol-doc-drift | Low | Native | FX5 | Implemented: HELPER_PROTOCOL.md matches the helper |
| testing-doc-stale | Low | Consistency | FX5 | Implemented: TESTING.md matches chords, fixtures, tiers |

**Not sent to a fix lane.** audit-status-never-updated (Low, consistency) found that the audit's §13 index still marked every finding Open and pointed to an implementation-status document that did not exist, so readers could not tell what had shipped. It concerns the audit document rather than code, so the integrator handles it directly: statuses are being written into the audit register as part of closing this cycle. The reviewer asked that UX-001, SEC-003 and SEC-004 read as partial until the ⌘R, privacy-eye and Cloud AI issues were fixed; FX4 and FX2 report those three as implemented (hud-local-cmd-r-dead-in-composer, privacy-eye-overclaims-after-sec004, cloud-ai-off-apple-server-speech).

**What the review changed.** Several of the material issues sat where two workstreams' changes met, which no single lane's tests could see. After every detected question the app stuck in Analyzing, because background snapshots still entered that phase and E1 had removed the events that left it (FX2). The pre-migration database backup that D2 added with the boot-failure fix (CRIT-003) was a full plaintext copy that outlived G's deletion and secure-delete work; deletions now drop it, and only the current version's backup is kept (FX2). Switching to `panic=unwind` for the PDF parser meant that any other panic in a task or command died with no log line or crash report and left the app half-working; a hook now logs it and aborts unless the panic is contained (FX3). A sign-out whose Keychain delete failed left the auth screen on an endless spinner and signed the user back in at the next launch; it now stays signed out and the delete is retried (FX3). In the engine, the PERF-006 relevance floor dropped the screen and conversation text that common asks were about, and the debug shape fired on any code that named an exception type (FX1). Three privacy and platform claims were also corrected: Cloud AI off still allowed Apple's server speech and Exa and Firecrawl research (FX2), the new ⌃⌥ window-move chords collided with Rectangle's and Magnet's default chords (FX2), and the HUD eye again claimed the full protection that SEC-004 had retracted (FX4).

## 7. Manual macOS QA: executed versus defined

The audit (§11) defines 29 manual checks, Q1–Q29. They were run on this Mac (Intel Core i5, x86_64, macOS 26.5.1) only where they need no sign-in, no live provider traffic, no second build or update, and no other person in a call. Bluey's sign-in gate (ONB-003) is in front of every ask and every listening run, and signing in needs the owner's browser account, so most checks stay **defined but not executed**.

**Smoke runs of the integrated build (2026-10-09, head `f324e14`: FX1–FX4 merged; FX5–FX7 were merged afterwards and are covered by their tests, not by these runs).** These are `tauri dev` builds with `BLUEY_DEV_SIGNING_IDENTITY=auto` and `BLUEY_DATA_DIR` pointing at scratch directories, so the installed app's data was not touched. Both runs exercise the boot path, migrations, signing and the first onboarding steps.
- **Run 1, upgrade.** It used a copy of the owner's 0.1.2 profile, made with `sqlite3 .backup`. The copy held migrations 0001–0004, 3 sessions, 27 transcript segments, 101 answers, 2 documents and 10 modes.
- **Run 2, fresh install.** It used an empty data directory and the same signed binary.

| Check | Result |
|---|---|
| Q1 Fresh install | **Executed up to sign-in (run 2).** Migrations 0001–0006 applied, with no backup because there was nothing to back up. The *Welcome to Bluey* window opened, then *Sign in* with *Sign in with your browser*, and *Continue* stayed disabled. Nothing was captured before consent: 0 screen snapshots, 0 transcript segments, 0 sessions, 0 AI requests, and an empty frames folder. The steps after sign-in were not reached. |
| Q2 Upgrade from existing data | **Executed (run 1).** Bluey wrote `bluey.db.bak-0.1.2`, then applied `0005_modes_lifecycle` and `0006_shortcut_defaults`. Session, segment, answer, document and mode counts were unchanged. All 10 built-in modes got a fingerprint and now match the shipped text; 4 of them took new instructions. The shortcut rows left the old global defaults: Move is ⌃⌥⌘ + arrows, Scroll is ⌘⌥↑/↓, and New Chat and Settings are no longer global. Quit from the app menu was clean: the WAL was checkpointed to 0 bytes and `integrity_check` returned ok. |
| Q20 Hotkeys | **Partly.** In the migrated profile, ⌘←/→/↑/↓, ⌘⇧↑/↓, ⌘R and ⌘, are no longer stored as global bindings. Typing in TextEdit or Safari while Bluey runs was not tried, because it needs real key events in the owner's session. |
| Q22 Bluey-owned credential after restart | **Partly.** Run 2 started the same signed binary. The `.env` import rewrote the dev items that run 1 had created, and no Keychain dialog appeared. A decrypting read of a saved key, which an ask would do, was not exercised. |
| Q24 Keychain under `tauri dev` | **Partly.** The dev binary was signed with the Apple Development identity under `com.codewithabdul.bluey.dev`, with a certificate-based designated requirement. Neither run raised a Keychain dialog (no `SecurityAgent` process). Sign-in and account tokens were reported *Absent*, because they live in the installed app's service, so the dev build never touched the installed app's items. The unsigned comparison was not run. |

Other observations from the smoke runs:
- **Accessory app (MAC-010).** `lsappinfo` reports `ApplicationType = UIElement`, so there is no Dock icon. The app menu keeps the Edit commands.
- **Helper start (MAC-011, MAC-012).** The helper was ready about 4 s after spawn, at a load average near 400, with no handshake timeout. For comparison, the installed 0.1.2 build's own log from the same morning shows `helper.version` timing out and the helper being restarted.
- **Frame sweep (DATA-001).** At startup the helper deleted 54 stale temp frames that the installed 0.1.2 build had left in the cache folder.
- **Log level.** Neither boot logged a warning or an error. Quitting logs one `helper exited` warning, because shutdown stops the helper; logging that at debug level is a follow-up.
- **HUD visibility.** The HUD stayed hidden in run 1 because the copied panel state was hidden. Bluey shows it at launch only when onboarding is complete and the saved state is visible.
- **`.env` import.** In both runs the `.env` import (debug builds only) copied the three keys from `.env.local` into the `.dev` Keychain service.
- **Shared data directory.** Debug builds share the data directory with the installed app unless `BLUEY_DATA_DIR` is set. `docs/DEVELOPMENT.md` now says so.

**Executed earlier in the cycle.**
- The non-interactive Keychain ACL probe (audit §4.2).
- The opt-in real-Keychain backend round trip, with user interaction disabled.
- The dev signing runner on real binaries (§2).
- The Swift helper XCTest suite, on this x86_64 Mac.

**Defined, not executed.**

| Checks | Why not |
|---|---|
| Q3, Q4, Q6, Q13, Q15, Q16, Q17, Q19, Q27, Q28 | Each needs an ask, which is behind the sign-in gate. Q28 is supported indirectly by the frame sweep above and by the frame-lifecycle tests. |
| Q5, Q9, Q10, Q11, Q12, Q21, Q29 | Each needs a listening run (sign-in), plus audio playback or a network monitor. Q5 depends on MAC-006, which is still open. |
| Q7, Q8, Q23 | These need the owner's ChatGPT, Claude or Antigravity accounts. Q23 also raises the owner's Claude Code Keychain dialog, which only the owner should answer. |
| Q14, Q18 | These need a real call with remote participants, and Zoom or Meet screen sharing. |
| Q25, Q26 | These need two signed builds, or a published ad-hoc update. Each build took about an hour on this machine under its load. |

When the owner runs these checks, the audit's §11 table gives the steps and expected results for each one. Q6, Q24 and Q26 confirm the credential fix end to end.

## 8. Remaining issues, known limitations, follow-ups

**Owner actions and decisions.**
- **Developer ID signing and notarization for Latest and Nightly (CRIT-001, MAC-001).** This is the root fix for the Keychain dialogs and the permission resets that follow every ad-hoc update. Until it ships, each update costs at most one approval per Keychain item actually used, and the repair card explains the permission resets.
- **Stable uploads only through the gated pipeline (DEBT-001).** Nightly already waits for green CI.
- **Sign-in policy (ONB-003).** Product policy: keep the sign-in, explain it, or make it optional. The sign-in gate also blocks most of the manual QA in §7.
- **Public history purge of `cluely-screenshorts/` (DATA-004).** This needs a history rewrite and a force-push.
- **First real release.** Confirm the `release.sh` split in a real run: the app builds with `--no-bundle` without the updater key, and only `tauri bundle` gets the key.
- **Checks that need the owner's accounts.** Run the opt-in live prompt eval (`BLUEY_PROMPT_EVAL_LIVE=1`) and the provider fingerprint captures (TEST-011).

**Status of all 247 findings.**

| Status | Count |
|---|---|
| Implemented | 128 |
| Implemented, needs real-device verification | 67 |
| Partially implemented | 12 |
| Deferred | 40 |
| **Total** | **247** |

Every entry in Appendix B and in `findings.json` now records its status and a note on what changed.

**Partially implemented.**
- **Credentials and signing.** CRIT-001 is short of Developer ID signing.
- **Context.** CTX-001 still has no recency limit on the fallback question.
- **Live loop and UX.**
  - AI-016: the research note is not saved with the answer.
  - UX-012: answers asked outside a session are not listed in History.
  - MAC-006: there is no repair flow for a permission revoked mid-session.
  - MAC-012: the backoff reset was not tested separately.
- **Providers.** FEATURE-004 and UX-034: key-prefix inference and a simpler Models section are not built.
- **Docs.** SEC-018: the docs are fixed, but the onboarding microphone copy still needs to say where audio goes.
- **Tests.**
  - TEST-004: there is no fake-helper process harness.
  - TEST-014 and TEST-015: settings defaults and provider presets are still hand-copied in the mock.

**Deferred.** The register gives the reason for each one. In summary:
- Performance work that needs a measured baseline first: PERF-007, PERF-009, PERF-010, PERF-013, PERF-014 and PERF-017.
- The per-window command ACL manifest (SEC-007).
- Echo cancellation and speaker attribution (MAC-008), which need real-meeting A/B tests.
- A Keychain vault (PERF-016).
- Test-infrastructure follow-ups: TEST-002, TEST-007, TEST-010, TEST-012 and TEST-016 to TEST-021.
- 16 low-severity or opportunity items that were never scheduled. Among them: MODE-008 (the summary uses the session's starting mode), UX-022 (onboarding still hardcodes shortcut keys), UX-029 (History gaps) and DEBT-013 (34 commands with no caller).

**Follow-ups found during the cycle.**
- **Data directory in debug builds.** Debug builds share the installed app's data directory, so `tauri dev` migrates the installed app's database unless `BLUEY_DATA_DIR` is set. This is now documented in `docs/DEVELOPMENT.md`. A separate `.dev` data directory, like the `.dev` Keychain service, would remove the trap.
- **Quit log noise.** Quitting logs a `helper exited` warning, because shutdown itself stops the helper. Logging that at debug level would make the warning meaningful.
- **Flaky UI tests under load.** `tests/ui/windows.test.tsx` (SettingsWindow General page) and a few other UI tests use default `waitFor` timeouts that fail on a loaded machine. They need explicit timeouts.
- **Rust test seams.**
  - The ResearchManager Cloud AI gate is covered by a mock-parity test only.
  - The Apple Speech reroute failure path has no test seam.
  - The Claude `agent.info` probe could be cached.
- **Regenerate on a ⌘⇧↵ suggested-response turn.** It still becomes a plain regenerate, and loses the spoken voice (FX1 note).
- **Saved credentials.** It offers no Remove action for tokens of a disconnected account. The next launch deletes them.
- **Tuning on real usage.** The audit-cycle constants should be tuned against real data: the 0.25 semantic floor, the 150 ms OCR deadline, the shouldSurface thresholds and the résumé pin size.
- **Side effects of the smoke runs.**
  - The two smoke runs wrote the three keys from `.env.local` into the `com.codewithabdul.bluey.dev` Keychain service. That is the normal debug `.env` import, and the items can be removed in Keychain Access.
  - The helper's startup sweep deleted 54 stale temp frames that the installed 0.1.2 build had left in its cache folder.
