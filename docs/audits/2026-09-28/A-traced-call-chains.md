# Appendix A — Traced call chains

Companion to [BLUEY_DEEP_AUDIT_2026-09-28.md](../BLUEY_DEEP_AUDIT_2026-09-28.md). Each auditor's end-to-end trace (UI → store → command → Rust → helper/sidecar/provider → event → UI) at `1a117a5`.

## Credentials & Keychain

## Secret store core
- `src-tauri/src/secrets/mod.rs:88-176` — `SecretsStore { service: BUNDLE_ID }` (storage/mod.rs:12 `com.codewithabdul.bluey`, shared by dev and prod). `entry()` (:113) builds a new `keyring::Entry` on every call; there is no cache.
  - `get_sync` :119-130 → `get_password()`; `NoEntry` → `Ok(None)`; any other `Err(_)` → `BlueyError::storage("keychain","failed to read from the keychain")`. The OSStatus is dropped and nothing is logged.
  - `has_sync` :132-134 = `matches!(get_sync, Ok(Some(_)))`, so it returns false on ANY error. Async `has` :168 = `get().is_some()`, so errors propagate.
  - `set_sync` :137-146 → `set_password` → "failed to write to the keychain". `delete_sync` :148-156 → `delete_credential` (NoEntry ok).
  - The allow-list `validate_key` :96-110 is covered by tests :244-288, which mirror `SECRET_KEYS` in src/lib/tauri/commands.ts:514-519.
- keyring 3.6.3 `src/macos.rs`: set :56-60 → `SecKeychain::set_generic_password`; get :79-83 → `find_generic_password` (SecKeychainFindGenericPassword WITH a data pointer, i.e. a decrypt that the ACL gates); delete :101-106 → find WITH data, then `item.delete()`. `decode_error` :257-265 maps only -25291/2/4/5 → NoStorageAccess and -25300 → NoEntry. Everything else (-25293 errSecAuthFailed, -128 errSecUserCanceled, -25308 errSecInteractionNotAllowed) becomes PlatformFailure.
- security-framework 3.7.0 `os/macos/passwords.rs:269-279` `set_generic_password`: `match find_generic_password(WITH data) { Ok → item.set_password (in-place modify), _ → add_generic_password }`. A denied find therefore falls through to add → errSecDuplicateItem → "failed to write".

## WebView → Rust chains
- Settings AI tab: `SecretKeyField.tsx:41-58` useEffect → `api.secrets.has` → `commands/settings.rs:48 secrets_has` → async `has` → data read. One per `ProviderCard.tsx:196` (every provider rendered by `AITab.tsx:298`) plus Exa/Firecrawl/Anthropic (`AITab.tsx:451-464`). On error it shows an empty field and a "failed to read from the keychain" toast.
- Save key: `commands/settings.rs:37-43 secrets_set` → `set` → `settings.refresh_provider_keys()` (settings/mod.rs:160-176). That runs a SYNC `has_sync` per provider while holding the `current.write()` lock, inside an async command.
- Any settings change: `settingsStore.ts:35` → `settings_update` → `SettingsManager::update` → `replace()` settings/mod.rs:150-156 → `has_sync` for every provider. Tray privacy toggle (platform/mod.rs:235), env import, reset and data reset take the same path.
- Auth status: 3 WebViews at boot (`bootstrap.ts:72 initStores` → `initStores.ts:71 auth.getStatus`) → `auth_get_status` → `auth/mod.rs:141 status()` → `has(auth:clerk:oauth_tokens)`.

## Boot (app/mod.rs)
- :130 `SettingsManager::load` → settings/mod.rs:38 `has_sync` per provider (sync, setup thread).
- :137 `env_import::import_env` → env_import.rs:79 `get_sync` / :97 `set_sync` per keyed env var (dev .env.local), then `update_sync` → `replace` → another N reads.
- :274 `auth.has_stored_session()` → auth/mod.rs:137 `has_sync(clerk)`. A false here (including on error) shows the sign-in gate.
- :362 `auth.restore()` → auth/mod.rs:516 `has(legacy client_token)` → :524 `load_tokens` (:482 get) → refresh if expiring → `fetch_user` → :543 `store()` → :495 `secrets.set(clerk)`. This rewrite happens on EVERY boot, even when the tokens did not change. Then `publish()` → `status()` `has`.
- :366 `accounts.restore()` → accounts/mod.rs:822-845 → `credential_for` for each connected account → :744 get + :789 set.

## AI request chain
engine.ts `runPipeline` (ask; `prepare()` at src/ai/engine.ts:664 for live suggestions; classify refine; session summary) → ai stream command → `ai/mod.rs adapter_for` :178-200:
- API-key providers: `self.secrets.get(provider:<id>:api_key)` on EVERY call (drive_provider, embed, transcribe_file, test_connection, list_models).
- OAuth-subscription providers: `accounts.credential_for` (accounts/mod.rs:768-792) → `token_cache` (:752-763; loads once per process via `load_tokens` :742-750, which uses `.ok().flatten()`, so errors are cached as None) → `cache.fresh(... profile.refresh)` → if `expires_at` is Some, `secrets.set(account tokens)` on EVERY successful call (:788-790, `let _ =` swallows errors).
- Router usability: `router.rs:256` enabled && has_api_key; `app/checks.rs:28`.

## Other readers
- research/mod.rs:65-69 Exa get per search, :98-102 Firecrawl get per scrape, :125-133 availability = has(Exa) + has(Firecrawl) + agent.available(has) (engine.ts:222-224, on every ask that wants research).
- agent/mod.rs:99-119 available (has), :122-158 job_env (get provider/Anthropic, Exa, Firecrawl); sidecar spawned with env_clear().envs(env) :233-234.
- audio/mod.rs:378-383 Gemini Live key, :412-417 cloud realtime key (once per listening start).
- documents/mod.rs:139-150 retrieve → ai.embed → adapter_for → one more key read per ask when embeddings are on.

## Deletes
- accounts/mod.rs:546 disconnect, :854 reset_all; auth/mod.rs:577-590 clear_session (load_tokens for revoke, then `delete(...)?` returns early on failure); commands/data.rs:62-80 reset (all keys). No UI calls `secrets.delete` (the api.ts:191 wrapper is unused).

## Foreign reads (import only, verified)
- accounts/claude.rs:295-310 `read_local_credentials` loops over candidate accounts ($USER, default, unknown) on service `Claude Code-credentials` (spawn_blocking at :383).
- accounts/antigravity.rs:450-455 `read_keychain_tokens` reads `gemini`/`antigravity` (bluey-protocols/src/antigravity.rs:99-100; spawn_blocking at :563).
- Both are reached only via `ProviderProfile::import` ← `AccountsManager::import` accounts/mod.rs:426-437 ← `commands/accounts.rs:38 accounts_import`. rg found no boot/restore/refresh/catalog caller. ChatGPT import reads ~/.codex/auth.json (chatgpt.rs:483-520).
- Later refreshes of imported Claude sessions use Bluey's own copy (claude.rs:468-509) and never write back to the foreign items.

## Signing / dev
- tauri.conf.json has no signingIdentity and no build.runner; there is no .cargo/config.toml. Tauri CLI dev runs `cargo run` (tauri-cli interface/rust/desktop.rs), so a cargo `[target.'cfg(target_os = "macos")'] runner` in src-tauri/.cargo/config.toml receives the built binary path as its last argument. Relative runner paths resolve against src-tauri/ (cargo config docs). The runner also applies to `cargo test` binaries.
- `codesign -dv target/debug/bluey` → 'code object is not signed at all' (Intel ld does not ad-hoc sign; Apple Silicon ld does, which gives a cdhash that changes per rebuild).
- release.sh:100-104 sets `APPLE_SIGNING_IDENTITY=-`; nightly.yml builds through release.sh; the repo has no APPLE_* secrets; gh releases v0.1.0-0.1.2 and nightly are labelled unsigned; UPDATES.md:54 checks 30 s after launch and every 6 h, then auto-installs.

## Security & privacy

### Capture and the frame cache (the ⌘↵ fast path)
- The chain:
  - `useAsk.ask` (src/features/hud/useAsk.ts:33-70) → `engine.runPipeline` (src/ai/engine.ts:271-314) → `buildNativeSnapshot` → `bluey.context.buildSnapshot`.
  - → Rust `context::build_snapshot_with` (src-tauri/src/context/mod.rs:88-141, which builds `ContextSnapshot { ..Default::default() }` and never sets `user_context`).
  - → `CaptureManager::capture` (src-tauri/src/capture/mod.rs:150-231).
  - → helper `capture.display` (ScreenCaptureService.swift:265-275: "The temp file is always written") → `~/Library/Caches/com.codewithabdul.bluey/frames/f-<uuid>.jpg` (TempFrames.swift:3-27).
  - → Rust `FrameCache`, capacity 8 (capture/mod.rs:31-72). Eviction removes only the map entry, not the file.
  - → `persist_snapshot` (capture/mod.rs:236-263 → snapshots.rs:36-39) stores `image_path` only when `store_screenshots` is on.
  - → OCR by path → base64 inline image → `PromptBuilder` → `ai_stream`.
- File deletion paths:
  - `capture_discard_frame` (commands/capture.rs:37; api.ts:72 `discardFrame`) has no caller in src/.
  - The helper deletes files older than 1 h, only at startup (HelperApp.swift:42 → TempFrames.swift:30-44).
  - `data_delete_screenshots` and `data_reset_all` only unlink DB-referenced `image_path`s (retention.rs:103-129; snapshots.rs:88-100).

### Research
- Web search: `engine.maybeResearch` (engine.ts:211-241) is called at engine.ts:330, before `enrichSnapshot` at engine.ts:333.
  - → `buildPublicQuery(instruction, nativeSnapshot)` (src/ai/research.ts:174-203; `privateTerms` reads `snapshot.userContext`, which is always undefined here).
  - → `research_search` → `ResearchManager::search` (src-tauri/src/research/mod.rs:60-91, Exa, `x-api-key` header).
  - Scrape: Firecrawl gets only Exa result URLs (research.ts:238-247; research/mod.rs:94-122 checks the http(s) scheme).
- Deep research: `runDeepAgent` (research.ts:256-297) sends `{query, goal, tools}` and never `allowedDocumentIds`.
  - → `AgentManager::start` (src-tauri/src/agent/mod.rs:199-268): `env_clear()` + `child_base_env()` (sidecar/mod.rs:416-430: PATH/HOME/TMPDIR/USER/LOGNAME/LANG/LC_ALL) + `job_env` (agent/mod.rs:123-159: one backend key, Exa/Firecrawl keys, passthroughs incl. `BLUEY_CLAUDE_CLI`, `BLUEY_AGENT_MOCK`).
  - → sidecars/agent/src/agent.ts:600-650. The Claude Code subprocess env comes from `buildSubprocessEnv` (agent.ts:295-345): provider vars rewritten, no telemetry opt-outs. Options include `tools: []`, `permissionMode: dontAsk`, `persistSession: false` and an empty temp `cwd`.
  - `document.request` is answered only for ids in the job allow-list (agent/mod.rs:355-382). In practice it is always rejected because the TypeScript side never passes ids.

### Audio
- `audio_start` → `AudioManager` → `cloud_provider` (src-tauri/src/audio/mod.rs:350-420).
- The Gemini Live branch (audio/mod.rs:369-389) needs only an enabled Gemini provider and a Keychain key. `privacy.cloud_ai_enabled` is never consulted (grep: no Rust reads outside types).
- → `GeminiLiveProvider` (transcription/gemini_live.rs:241-320): the key sits in the `?key=` URL; log lines use `redacted_url()`.
- The default `transcription_provider` is `GeminiLive` (bluey-core types/settings.rs:191).

### Secrets and the IPC boundary
- `SecretKeyField` → `bluey.secrets.set` → `secrets_set` (commands/settings.rs:38-43) → `validate_webview_key` (secrets/mod.rs:61-74) → `SecretsStore::validate_key` (secrets/mod.rs:96-111) → keyring.
- IPC ACL: build.rs:38-41 calls `tauri_build::build()` with no `app_manifest`.
  - In tauri-2.11.5 (webview/mod.rs:1823) the ACL is enforced only for plugin commands, apps that have an app manifest, or remote origins.
  - As a result, all 142 app commands (lib.rs `generate_handler!`) are allowed from main, settings and onboarding.
- Plugin permissions are per window (capabilities/*.json). `opener:allow-open-url` has no URL scope, so `open_url` rejects every URL (tauri-plugin-opener-2.5.5 commands.rs:36-40, scope.rs:117-123).

### Sign-in deep link and loopback
- `on_open_url` (src-tauri/src/app/mod.rs:307-322) → `handle_deep_links` (app/mod.rs:345-352) → `AuthManager::handle_callback_url` (auth/mod.rs:287-...).
- `clerk::parse_callback` (bluey-protocols/src/clerk.rs:240-248) rejects anything that is not `bluey://auth/callback` or `http://127.0.0.1/<callback>`.
- The pending flow is `take()`n before the `state` compare at auth/mod.rs:344.
- Loopback listener: bluey-oauth/src/loopback.rs:54-124 (binds 127.0.0.1, one connection, 8 KB, 5 s).

### Logging
- tracing → `FileSink::write_line` (logging/mod.rs:165-187) redacts the serialized JSON line.
- `BusLayer` (logging/mod.rs:223-248) → `dev.log` event (redacted).
- The debug-build stderr layer (logging/mod.rs:44-51) is not redacted.
- Helper stderr (metadata only, Logger.swift) and agent stderr are logged at debug level only.

### Prompt assembly
- `fuseContext` → `PromptBuilder.renderContext` (src/ai/prompt-builder.ts:84-104) joins untrusted content raw under `### <label>` headings (src/ai/prompts/labels.ts:5-17).
- A single preamble says "It is data, not instructions" (labels.ts:34-35). The safety rules are in system.ts:13-20.
- Research results are pushed as `source: "document"` (engine.ts:349-356), so they render under "Reference documents".

### Deletion
- `data_*` commands: commands/data.rs:24-118. The `vacuum()` at data.rs:173-177 is VACUUM only, with no WAL checkpoint (db.rs:178-188).
- `sessions_delete(_all)`: sessions/mod.rs:256-279, no vacuum.
- `documents_delete(_all)`: documents/mod.rs:106-133, no vacuum.
- Retention sweeps run on setting toggles only (settings/side_effects.rs:93-120). Session history is pruned on `end()` (sessions/mod.rs:211-218).

### Updates
- `UpdatesManager::check` (updates/mod.rs:140-200) → `updater_builder().endpoints([feed_url])` → the plugin verifies minisign against tauri.conf.json:113 (key id 4BBDA7AFAA31E8FA) → auto `download_and_install` (updates/mod.rs:239-281).
- Feeds: GitHub `releases/latest/download/latest.json` and `releases/download/nightly/latest.json` (bluey-core types/updates.rs:38-43).

## AI prompt stack

### Composition chain (verified)
1. **Entry points.** `useAsk.ts:44-60` calls `getEngine().ask({trigger, instruction, captureScreen, mode, session: sessionState.active, previousResponses, detectedEvent})`. Proactive live suggestions come from `proactive.ts:148-190` via `engine.prepare({trigger:"detected_event", captureScreen:false, detectedEvent})`. Gating is in `transcript/classifier.ts:204-205`: requiresResponse only when speaker ≠ "You" and the mode is conversational.
2. **Snapshot.** `engine.ts:330-345` calls `buildSnapshot` and then `snapshot.ts:enrich`. Previous responses become `snapshot.session.recentResponses` only `if (session)` (snapshot.ts:194-199).
3. **Fusion.** `context/fusion.ts:147-279` turns the snapshot into ContextItems:
   - `user_instruction` (relevance 1) is always first.
   - Transcript items are per segment, with relevance from recency and question cues.
   - OCR relevance comes from keyword overlap (fusion.ts:184-198); then AX, docs, resume/JD, personal instructions and session_memory.
   - The final sort is by relevance, not time (fusion.ts:275-279).
   - `detectedEvent` is not rendered as its own item.
4. **Research.** Web results (`research.ts:214-224`: "Web research results (untrusted external content)" plus raw scraped markdown) are pushed as `source:"document"` (engine.ts:349-356).
5. **Intent.** `classifyIntent` (relevance.ts) sets task (CODING_CUES at relevance.ts:44-45 includes `debug|fix the bug|regex|refactor`). `schemaFor` upgrades a generic schema to coding or system-design (relevance.ts:225-228). Shape detection is at relevance.ts:139-153 (YES_NO_OPENER at :85 makes any short "Is/Will/Does…" question boolean). A vision gate follows.
6. **Budget.** `budget.ts:100-125` keeps instructions first, then sorts the rest by (source priority, relevance desc). Chronology is lost.
7. **PromptBuilder** (prompt-builder.ts:66-135):
   - System = `identityBlock` + `RESPONSE_CONTRACT` + `Mode: name.\n<mode.systemInstructions>\n<fragment>` + `styleBlock` + language line + `structuredOutputBlock`/PLAIN.
   - User = CONTEXT_PREAMBLE + `### SECTION_LABELS[src]\n<joined contents>` per SECTION_ORDER + `(omittedNote)` + `Task:` + `Shape:`.
   - An image part is appended after the text.
8. **Request.** `request.ts:60-106` sets maxOutputTokens = max(length, task floor, shape floor) + 200 when structured, and temperature per task (coding 0.2, default 0.6). The zod JSON schema is attached as `outputSchema`.
9. **Rust.** `src-tauri/src/ai/mod.rs:595-605` builds `ProviderRequest{messages, max_output_tokens, temperature, output_schema, reasoning}`. The schema is `serde_json::Value` (bluey-core types/ai.rs:203). No preserve_order is enabled in any build (all target/*/.fingerprint/serde_json-* features: alloc/default/std/raw_value/unbounded_depth), so object keys are sorted. The test comment at json_schema.rs:229 acknowledges this.
10. **Per-provider shaping (bluey-protocols):**
   - **Gemini** (`gemini.rs:246-310`): system → `systemInstruction`; schema → `generationConfig.responseJsonSchema` (strip_meta only); image → `inlineData`.
   - **Antigravity** (`antigravity.rs:714-760`): the same, but `systemInstruction.role="user"`; the Antigravity identity text is injected only in the probe (806-817).
   - **OpenAI-compatible and Azure** (`openai.rs:52-94`, `azure.rs:78-86`): `system` role message; `response_format.json_schema` with `strict_variant` (all properties required, optionals nullable, additionalProperties false; min/max kept); `max_completion_tokens`, `temperature`; no reasoning_effort.
   - **Codex / ChatGPT** (`codex.rs:676-745`, `chatgpt.rs`): Bluey system → `instructions` (InstructionsPolicy::Own). A sticky fallback switches to the Codex CLI template, with Bluey sent as a `developer` input item. `text.format` is strict json_schema.
   - **Anthropic API key** (`anthropic.rs:44-99`): system → top-level `system` string; schema → `output_config.format` (strip_meta; keeps minimum/maximum/const). On a 400 mentioning output_config (`is_output_config_rejection` at :314), it retries with the schema text appended to system (providers/anthropic.rs:178-196). The retry is per request, not sticky.
   - **Claude subscription** (`claude_code.rs:603-606, 672-700`): `system[]` = billing header + "You are Claude Code…"; Bluey's system text becomes `<\system-reminder>\n…\n<\/system-reminder>` prepended to the first user text block, so it sits in the same block as the untrusted context.
11. **Stream.** The draft is `extractPartialStringField(accumulated,"content")` (engine.ts:465), so nothing is painted until the `content` key streams.
12. **Finalise.** `parseStructuredOutput` (schemas.ts:283-369: fence strip, trailing-comma repair, embedded object, double-encoded unwrap, partial-content salvage, prose fallback) → `BlueyResponse` → `optimizeResponse` (optimizer.ts:232-284: paragraph dedupe, `stripFillerOpeners` on the first prose piece for every shape, prose cap except spoken/written/code, `code` derived from the first fence for coding) → HUD `ResponseView.tsx:121-150`. The JSON guard `looksLikeStructuredJson` (partial-json.ts:74-84) only recognises envelopes containing `"responseType|content|sections|title"`. `code` is hidden when content already includes it (ResponseView.tsx:148).

### Where chains stop
- **Follow-up history** stops at snapshot.ts:194 when no session is active. Sessions start only from SessionMenu (session-actions.ts:27) or when listening starts (audio/mod.rs:315-317).
- **Detected-event question text** stops at fusion.ts:150 (only `instruction` becomes a section). The task line still says (see "Current question") (task.ts:20).
- **Anthropic structured output with a numeric range** likely stops at the API (400), then falls back to the schema in the prompt.

## Mode system

### Traced chains (verified unless noted)

**Seed / CRUD**
- `app/mod.rs:161-169` → `ModeManager::load` (`src-tauri/src/modes/mod.rs:28-54`) → `ModeRepository::seed_built_in` (`crates/bluey-storage/src/repositories/modes.rs:90-140`).
  - The seed is `ON CONFLICT DO UPDATE SET built_in=1, sort_order` only, so edits are preserved and new shipped text is never applied.
  - Active mode = stored `ACTIVE_MODE_KEY` (`repositories/settings.rs:48-62`), else `general.defaultModeId`, else `general` when the stored mode is gone (mod.rs:40-43).
- Commands (`src-tauri/src/commands/modes.rs`, registered at `lib.rs:122-123`) ← `src/lib/tauri/api.ts:135-136` ← `ModeEditor.tsx` / `ModesTab.tsx` / `hud/ModeMenu.tsx:41-46` / `onboarding/steps/setup.tsx:28` / `GeneralTab.tsx:83-88`.
- `create`/`update` go straight to the repository with no validation (mod.rs:106-126; repositories/modes.rs:191-307). `update` treats `None` as "unchanged", so group and preferredModelRole cannot be cleared (L295-302).
- Every mutation publishes `modes.changed`, which updates `useModesStore.applyRemote` (`initStores.ts:38`). `set_active` persists the id, runs `hub.transition(ModeChanged)` and publishes `mode.changed` (mod.rs:178-196). `mode.changed` has no TS subscriber; the UI reads `status.modeId`.

**Ask path (mode → behaviour)**
1. `useAsk.ts:39` and `proactive.ts:107-112` resolve the mode with `modeById(status.modeId) ?? default ?? modes[0]`.
2. `engine.ts` runs the pipeline:
   - **Capture:** `snapshotOptionsFor` (`src/context/snapshot.ts:60-80`) captures the screen when the mode requires `screen`, including proactive `detected_event` asks. AX = screen OR `accessibility`. Transcript = `transcript` OR the spoken triggers.
   - **Rust snapshot:** `src-tauri/src/context/mod.rs:203-213` attaches the active mode plus the effective style. `user_context` is never filled.
   - **Retrieval:** `retrieveRelevantContext` (`src/context/retrieval.ts:99-126`) → `inferKinds` (L73-93). It returns [] unless the mode requires documents, resume or job_description, and adds `notes`/`other` only for `documents`. The kinds filter is hard in Rust (`documents/retrieve.rs:59`, `repositories/documents.rs:118-160`), and scope is matched on `documents.scope_id`.
   - **Fusion:** `fusion.ts:249-261` always adds session memory; the `session_memory` requirement is never checked.
   - **Intent:** `classifyIntent` (`src/context/relevance.ts:263-317`):
     - task = `defaultTaskFor(mode)` (registry.ts:57)
     - the system-design schema forces `system_design` (L279); the coding schema forces `coding` (L293)
     - `schemaFor` upgrades only answer and suggested-response (L222-229)
     - reasoning: case → light (L205-210)
     - latency = `slowerOf(mode.preferredLatency, taskMinimum)` (L311)
     - shape: `detectAnswerShape` (L165-180) puts coding/design before spoken
   - **Prompt:** `PromptBuilder.renderSystem` (`src/ai/prompt-builder.ts:66-81`) = identity + SAFETY_RULES, RESPONSE_CONTRACT (`prompts/system.ts:28-36`; the precedence line gives mode instructions equal authority), `Mode: ${name}.\n${instructions}\n${MODE_PROMPTS[schema]}` (`src/modes/prompts/index.ts`), `styleBlock` (`prompts/style.ts:10-28`), language, `structuredOutputBlock`.
   - **Output:** `outputSchemaFor(schemaId)` is always sent (engine.ts:376). `parseStructuredOutput` falls back to prose (`schemas.ts:336-369`).
   - **Request:** `buildAIRequest` (`src/ai/request.ts:85-107`) carries no mode id or role.
3. Rust `AiManager::select` (`src-tauri/src/ai/mod.rs:231-244`) takes `preferred_role` from `modes.active_mode()` at routing time → `router::desired_role` (`bluey-core/src/router.rs:141-231`):
   - answer at ultra-fast/fast → fast role; balanced/deep → default role
   - the mode role overrides the derived role for text-generation tasks; vision required always wins
4. Effort mapping: `bluey-protocols/src/codex.rs:790-815` and `claude_code.rs:478-490`.

**Proactive path**
- `transcript.final` → `proactive.ts:212-230` → `engine.classify` (`engine.ts:799-820`) → `classifySegment` (`src/transcript/classifier.ts:176-216`):
  - schema-family bonus (L84-92)
  - `requiresResponse` only when `conversationalMode` (L108-115): candidate modes, sales, recruiting, suggested-response
  - otherwise the event is returned and the caller ignores it
- `question.detected` → `prepareFor` (`proactive.ts:148-196`) captures the mode once at start. The prepared cache is keyed by event id only (`engine.ts:664-740`; TTL 3 min at L137).

**Session / summary**
- `SessionManager::start` (`src-tauri/src/sessions/mod.rs:80-95`) fixes `session.mode_id` when the session starts.
- `set_active` never touches the session. `TimelineEventType::ModeChanged` (`bluey-core/src/types/session.rs:50`) is never written.
- `SessionDetail.tsx:178,245-261` → `engine.summarizeSession` → `generateSessionSummary` (`src/sessions/summary.ts:128-160`) → `summaryTaskFor(mode.responseSchema)` (`src/ai/prompts/summary.ts:12-37`).

**Attachments**
- `ModeFilesDropzone.tsx:10` (kind always `notes`) → `documents.add` → `DocumentManager::add` (`src-tauri/src/documents/mod.rs:55-72`) writes `documents(scope='mode', scope_id)` plus a `mode_documents` row.
- The list and retrieval use `scope_id`. `mode_documents` only feeds `attachedDocumentIds`, which no TS code consumes (only typed in `src/lib/types/mode.ts:63`).

**TS duplication of built-ins**
- `src/lib/tauri/mock/fixtures.ts:168-285` (MODE_SEEDS)
- `tests/fixtures/*/mode.json`
- `tests/fixtures/helpers/builders.ts:103` (`makeMode`)
- `src/lib/types/mode.ts:83` (`BUILT_IN_MODE_IDS`, used only as a type)
- `src/modes/registry.ts:17-31` (`CANDIDATE_MODE_IDS`)
- `src/ai/research.ts:78,107` (a second `isCandidateMode`)
- `src/transcript/speaker.ts:15-21` (id checks)

None of them is compared with the Rust `SPECS`.

## Context engine

### Main ask chain (verified)
1. The UI calls `useAsk().ask` (src/features/hud/useAsk.ts:33-71). It sets `previousResponses = completedResponses(chat.turns)` (chatStore.ts:184), `session = sessionStore.active` and `sessionEvents`. No notes and no transcript override are passed.
   - Live questions take a different path: `proactive.prepareFor` (src/stores/proactive.ts:148-178) → `engine.prepare({trigger:"detected_event", detectedEvent})`. It passes **no** previousResponses.
2. engine.ts:290-301 → `buildNativeSnapshot` → `snapshotOptionsFor` (snapshot.ts:60-88):
   - screen is included if the trigger is `shortcut_capture` or the mode requires screen;
   - transcript is included if the mode requires it or the trigger is `shortcut_generate`/`detected_event`;
   - `inlineImage = includeScreen` and `changeDetection:false`.
3. Rust `build_snapshot_with` (src-tauri/src/context/mod.rs:81-252):
   - `tokio::join!` runs frontmost app (`ax.frontmost`), AX snapshot (`ax.snapshot`, helper AXSnapshotService.swift: depth 6, 150 elements, focused value ≤4000 chars, element value ≤1000, visibleText ≤8000) and capture→OCR (`capture/mod.rs:306-348`; OCR cache bypassed because `changed=true`).
   - Transcript comes from `core.audio.recent(window)` (audio/mod.rs:672-682): finals only, cutoff relative to the **newest segment**, no session filter.
   - `session_context` (context/mod.rs:256-296) makes 4 DB queries: responses, events, notes, session documents.
   - Then `trim_snapshot` + `apply_adapter` (bluey-core context.rs:45-90, 193+). The image is stripped from the bus copy (242-246).
4. engine.ts:319-326 → `retrieveRelevantContext` (retrieval.ts:99-126). The query is built from instruction + last question-like transcript line + OCR headline. The `detectedEvent` is ignored. `inferKinds` returns [] unless the mode requires documents, resume or job_description. The call goes through documents_retrieve (src-tauri/src/commands/documents.rs:52) → `DocumentManager::retrieve` (src-tauri/src/documents/mod.rs:139-163; embeds the query only if embeddings are ready) → `bluey_storage::retrieve` (retrieve.rs:60-152): FTS5 OR-match (fts.rs:19-40) with bm25 normalised so the best hit scores 1.0, plus semantic cosine with no threshold, then scope/kind boosts and truncation to 8.
5. engine.ts:333-343 → `enrichSnapshot` (snapshot.ts:163-209). Session context is built only `if (session)`. `recentResponses` = last 5 turns, content cut to 320 characters, `code` and `prompt` dropped. It **overrides** the Rust DB responses (`previousResponses ?? []`).
6. engine.ts:345-357 → `fuseContext` (fusion.ts:147-279):
   - only the instruction becomes `user_instruction`;
   - each transcript segment is scored by age decay, with questions boosted and capped at 0.95;
   - OCR base 0.35 + keyword overlap; AX selected 0.9, focused 0.85, visibleText 0.55 (skipped only when *identical* to OCR);
   - chunks use their retrieval score; session_memory 0.4-0.6; personal 0.9;
   - items are sorted by relevance. `activeApplication`, `activeWindow`, session notes and events are never read.
7. engine.ts:359-366 → `classifyIntent` (relevance.ts:264-318). Coding/assessment/shape detection reads **only** `ocr.text`. `visionRequired = hasScreen && (ocr+ax chars < 200 || visual cue in the question || coding with low OCR confidence)`.
8. engine.ts:368-370 → `allocateBudget(items, contextTokenBudget - headroom)` (budget.ts:99-178): instruction first, then priority tier, then relevance. There is no relevance floor and no per-source cap. The Rust `bluey_core::budget` is not used at runtime.
9. engine.ts:385-399 → `PromptBuilder` (prompt-builder.ts:84-134): items are grouped by source in `SECTION_ORDER` (prompts/labels.ts) and each bucket keeps budget order, i.e. relevance order. `detectedEvent` is accepted but never rendered. The message list is [system, user]; no assistant history.

### Transcript producers
- Helper transcript times are ms since `audio.start` (SpeechTranscriber.swift:10; AUDIO_ARCHITECTURE.md:77). `TranscriptAssembler::assemble` (bluey-protocols helper.rs:560-600) keeps the partial id for the final, sets `speaker_label(source, mode_id)` (helper.rs:464-481) and does **not** rebase times.
- `commit_final` (audio/mod.rs:911-930) pushes into the ring (capacity 500). `start()` (240-267) resets assembler/partials/chunk_times but not the ring.
- In the UI, `transcriptStore.applyFinal` replaces a partial by id. `proactive.onFinal` → `engine.classify` → `classifier.classifySegment` works on single final segments.

### Screen change detection
- Smart observation: app/mod.rs:393-397 → helper ScreenObserver → sidecar/mod.rs:211-217 publishes `BlueyEvent::ScreenChanged` → no subscriber in Rust or TS.

### Rust/TS parity
- TS budget.ts: priority tiers, no caps; the only runtime budgeter. Rust bluey-core budget.rs: priority×relevance score with a `max_share` per source; tests only.
- Speaker labels: Rust hardcodes mode ids; TS `counterpartLabelFor` uses `isCandidateMode` and lecture→"Lecturer".
- Mock fixtures (src/lib/tauri/mock/fixtures.ts) differ from the Rust seeds: budget 24k vs 12k (settings.rs:287), and different `contextRequirements` for general/interview/case-interview.

## Live suggestions & real-time races

### Detection → preparation
- `transcript.final` (Rust `audio/mod.rs:881-929` `commit_final` publishes each final. Default STT is Gemini Live with server VAD `SERVER_SILENCE_MS = 600` at `transcription/gemini_live.rs:47-48`; Apple Speech rotates about every 55 s, `SpeechTranscriber.swift:9,35`) → `proactive.ts:210-227 onFinal` → `engine.classify` (`engine.ts:799-818`) → `classifySegment` (`classifier.ts:176-217`: finals only (:180), min confidence 0.5 (:168), `requiresResponse = RESPONSE_WORTHY && speaker !== "You" && conversationalMode(mode)` (:204-205), random `evt_` id (:208)) → optional fast-model refine only when 0.4 ≤ conf ≤ 0.7 (`engine.ts:811`) → `bus.emit("question.detected")` (:816).
- `proactive.ts:199-208 onDetected`: HUD window only; `proactivePreparation` flag; dedupe by event id; if `busy`, the event goes to `queued` (newest wins); otherwise `prepareFor`.
- `prepareFor` (`proactive.ts:148-197`): `canShowLive(settings, chat.phase)` (:91-93) → live: `chat.begin(event.text, event.text, {phase:"thinking", suggestion})` (:162) plus `liveCallbacks` (:115-128), which call setPhase/markCancelled, applyDraft, complete + `consumeQuestion`, fail. The input carries mode, session, settings and sessionEvents but **no previousResponses** (:166-177). `finally`: busy=false, then run `queued` (:189-196).

### Generation
- `engine.prepare` (`engine.ts:664-719`): live = `callbacks.onComplete !== undefined`; `gate.next(SCOPE_PREPARE)` (:679); `isCancelled: () => false`, `onStreamHandle: () => {}` (:686-687). There is no handle, so there is no cancel path.
- `runPipeline` → `buildAIRequest` (generation = per-scope counter, `sessionId` whenever a session is active: `request.ts:102`) → `streamRequest` (`engine.ts:451-484`); drafts come from `extractPartialStringField(accumulated,"content")`; one retry on `finishReason: length` resets the draft (:450-453, 492-504) → parse (:530-536, raw JSON never shown) → persist + `response_generated` session event unless silent (:566-588).
- `engine.ask` (`engine.ts:595-648`): `gate.next(SCOPE_ASK)`, cancels only the previous ASK in-flight (:600-601), returns a handle with `cancel`.
- Per-scope counters: `generations.ts:4-17`. ADR 0005:21-24 documents the per-scope gate.

### Rust
- `ai_stream` → `AiManager::start` (`src-tauri/src/ai/mod.rs:266-310`): supersede cancels **any** active entry with the same `session_id` and a lower `generation` (:281-292), with no scope awareness. Every primary task triggers `ThinkingStarted` (:304-306; `is_primary` :996-1001).
- `run_stream` (:430-575): Completed Stop/Length → `ResponseReady` (:521-523); Cancelled → no state transition (:525-540); Failed → `publish_failed` → `AppEvent::Failed` for primary tasks (:678-686). There is no provider fallback mid-stream (`drive_provider` :577-676).
- State machine (`bluey-core/src/state/mod.rs:224-247`): ThinkingStarted is allowed from Ready/Listening/Analyzing/ResponseReady; ResponseReady only from Thinking; ResponseDismissed only via `app_dismiss_response` (`commands/app.rs:47-53`, which also calls `ai.cancel_all()`), whose only TS caller is `useAsk.newChat` (`useAsk.ts:128-132`). Error state rejects audio toggles (`state/mod.rs:291-300`).

### UI
- `HudPanel.tsx:36` sets `expanded = turns.length > 0` and conditionally mounts `FollowUpHeader` or `HudIdleRow` (:118-140). Each has its own `useHudInput` `useState("")` plus autofocus on mount (`HudInputRow.tsx:14-30`). `submitTyped` sends `follow_up` without screen once expanded (:42-49).
- Escape → `onEscape` (:65-68) → `useAsk.stop()` (`useAsk.ts:73-83`: `markCancelled` + `currentHandle?.cancel()`, where currentHandle is the last **ask** handle).
- ⌘⇧↵ → `generateOrTakePrepared` (`useAsk.ts:89-115`) → `chatStore.showResponse` (`chatStore.ts:150-169`: bumps generation, does not cancel streaming turns, does not persist).
- `response.prepared` → `initStores.ts:58` → `chatStore.prepared` (no TTL) → `derivePill` (`state-pill.ts:36-69`: error > busy phase / `status.state === "thinking"` > prepared > preparing > listening).
- `ResponseThread.tsx:79-103` renders a turn with status `streaming` as live forever. `Regenerate` → `useAsk.regenerate` (:117-126) re-asks `last.prompt` as a typed instruction.

### Persistence
- The only save site in src is `engine.ts:569` (non-silent pipelines). Live suggestions are saved (including cancelled ones that keep streaming). Silent-prepared responses shown via ⌘⇧↵ are never saved.

## Providers, routing & accounts

### Main text-generation chain
- UI → `src/ai/engine.ts` → `buildAIRequest` (`src/ai/request.ts:85-107`: task, latency, reasoning, visionRequired, a temperature per task (0–0.6), `outputSchema` whenever the mode supplies one; `useStructuredOutput` is never set by any caller) → `bluey.ai.stream` → `#[tauri::command] ai_stream` (`src-tauri/src/commands/ai.rs:19`) → `AiManager::stream/start` (`src-tauri/src/ai/mod.rs:249-310`).
- `start` → `AiManager::select` (`ai/mod.rs:231-244`) → `router::select(input, settings.ai.models, self.providers())` (`router.rs:61-139`).
  - `providers()` (`ai/mod.rs:144-166`) = settings providers + `AccountsManager::provider_configs()` + dev mock.
  - For an account, `has_api_key = enabled && is_usable` (Connected, or RateLimited past `until`) (`bluey-core/src/accounts.rs:189-207`).
- `select` order:
  1. Explicit override (`router.rs:69-88`). `modelOverride` is typed in `src/lib/types/ai.ts:121` but never set by the UI.
  2. `fallback_chain(role)` (`router.rs:239-253`), skipping unassigned, unconfigured or unusable providers.
  3. Otherwise `config.no_model` (`router.rs:131-138`).
  - `provider_supports_vision` returns true for every kind (`router.rs:36-47`).
- A failed select publishes `AiFailed` (`ai/mod.rs:274-280`).
- `run_stream` (`ai/mod.rs:430-575`) sends `AiChunk::Started{selection}` (selection includes `reason`) → `drive_provider` (`577-676`) → `adapter_for` (`178-207`: API key from Keychain, or `AccountsManager::credential_for` for OAuth) → `build_provider` → `adapter.stream`.
- Terminal chunk → `note_provider_outcome` (`213-228`, OAuth providers only) → `AccountsManager::note_request_error/success` (`src-tauri/src/accounts/mod.rs:715-740`) → `rules::status_after_error` (`bluey-core/src/accounts.rs:109-150`).
- On the WebView side, `src/ai/stream.ts:145-146` calls `onStarted`, but no caller passes it; `src/ai/metrics.ts:25-27` keeps only providerId and model.

### Error mapping per adapter
- Gemini:
  - `send_with_retry` (`src-tauri/src/ai/providers/gemini.rs:112-170`): 3 attempts on 408/429/5xx, honours `retryDelay`, gives up at once on a daily quota.
  - `map_gemini_error` (`bluey-protocols/src/gemini.rs:624-680`): API_KEY_INVALID → `config.api_key_invalid`; 404 → `config.model_not_found` with ConfigureProvider; 429 carries `retryAfterMs` / `dailyQuota`.
- OpenAI-compatible (`providers/openai.rs:59-61`), Azure (`providers/azure.rs:97-99`) and Anthropic API key (`providers/anthropic.rs:157-161`) all use `map_http_status` (`providers/mod.rs:368-385`), which drops the body. 401/403 → ConfigureProvider; 429 → `network.http_429` (Retry, no retryAfter); anything else → `ai.http_<status>`.
- Anthropic OAuth → `claude_code::map_error`; Codex → `codex::map_error`; Antigravity → `ag::map_error`. Each maps 401 → `account.needs_reauth` (`claude_code.rs:976`, `codex.rs:1447`, `antigravity.rs:1067`).
- Antigravity retries using retry_after (`providers/antigravity.rs:132-144`).
- The HUD copy comes from `src/lib/errors/present.ts`; the generic `ai` kind message is "The model didn't answer. This is usually temporary."

### Accounts
- Connect / import / catalog: `AccountsManager` (`accounts/mod.rs`).
  - `refresh_catalog` (583-624) → `apply_catalog_presets_after_fetch` (642-661, overwrite=false: fills unassigned roles and re-points roles whose model left the catalog).
  - `apply_presets` (664-691, overwrite chosen by the caller).
  - `disconnect` → `unassign_roles` (538-580).
- `credential_for` (768-810) → `TokenCache::fresh` (`bluey-oauth/src/tokens.rs:101-124`): single-flight refresh, driven only by the clock (`is_expiring`). No refresh is forced after a request-time 401.
- HUD recovery: `reconnect_account` → `bluey.accounts.connect`; `use_api_key` → opens Settings → AI (`present.ts:320-335`).

### Settings UI
- `AITab.tsx`:
  - Default-provider select → `switchDefaultProvider` (233-249) → `ai_apply_provider_presets(overwrite:true)` → `presets::apply_presets` (`bluey-core/src/presets.rs:228-260`) or `AccountsManager::apply_presets`, then sets `bootstrapProvider`. The router never reads `bootstrapProvider`; only the env import does (`presets.rs:373,429`).
  - `ModelRoleRow` (57-178): provider select plus free-text model with a datalist from `ai_list_models` (`ai/mod.rs:919-937`, 120 s cache).
- `ProviderCard.tsx`: Edit, enable switch, key, "Use recommended models" (overwrite:true), Test connection → `ai_test_connection` (`ai/mod.rs:803-883`: 20 s timeout, temperature 0.0, max 64 tokens).
- `AccountsSection.tsx:55-58`: "Use recommended models" for an account = overwrite:true.

### Transcription
- `AudioManager` start → `route_for` (`src-tauri/src/audio/mod.rs:75-96`) plus `cloud_provider` (350-430): GeminiLive needs an enabled Gemini provider with a key; CloudRealtime needs a Foundry or OpenAI-compatible transcription assignment on a non-OpenAI-realtime transport. If the cloud route isn't ready it uses Apple and publishes `audio.stt_fallback` (334-339).
- Runtime: GeminiLive and Voice Live reconnect internally (`transcription/gemini_live.rs:53-57`, `transcription/cloud_realtime.rs:189-260`). After that, `TranscriptionEvent::Failed` → the source is marked failed (`audio/mod.rs:507-514`) and `forward_pcm` drops its audio from then on (448-449).
- Batch import: `ai_transcribe_file` → `AiManager::transcribe_file` (`ai/mod.rs:731-786`), Gemini or Mock only.

### Research
- `ResearchManager::deep_start` → `AgentManager::start` (`src-tauri/src/agent/mod.rs:199+`).
  - `job_env` (123-158): a Gemini key from the first enabled Gemini provider, or the Anthropic key from a separate keychain entry (`agent:anthropic:api_key`) or the env.
  - `research_model` (165-185): uses the research-role assignment only when its provider kind matches the backend.
- Inline "research" answers go through the router's Research role instead.

### Embeddings
- `AiManager::embed` (`ai/mod.rs:710-725`) reads `ai.models.embedding` directly; it does not go through the router or check that a key exists.
- Settings side effect (`settings/side_effects.rs:66-80`) → `DocumentManager::reembed_stale` (`documents/mod.rs:248-290`; only Gemini vectors are MRL-truncated to `embeddingDimensions`).

## Onboarding & settings coherence

### Boot → auth → windows
- `src/main.tsx` → `src/lib/tauri/bootstrap.ts` → `initStores` (src/stores/initStores.ts:74-84 loads auth/accounts/settings/app/modes/permissions/panel/session/updates; :36-40 subscribes `app.state`, `permissions.changed`, `auth.changed`).
- Rust `app::setup` (src-tauri/src/app/mod.rs): `authenticated = !auth.auth_required() || auth.has_stored_session()` (:274; `has_stored_session` = `secrets.has_sync(CLERK_OAUTH_TOKENS_KEY)`, which reads the password data, secrets/mod.rs:132) → `panel.attach(onboarding_completed)` (overlay/mod.rs:112, HUD not shown while onboarding) → `open_window("onboarding")` if not completed (:326) → `hub.transition(BootCompleted{authenticated})` (:335; state/mod.rs:165-173). `finish_boot` spawns `auth.restore()` (:362; auth/mod.rs:515-572: refresh if expiring, sign out on an auth error, keep the cached user when offline) and `permissions.refresh()` (:384).
- Deep link `bluey://auth/callback` → `handle_deep_links` (app/mod.rs:343-351) → `AuthManager::handle_callback_url` (auth/mod.rs:287-325); on failure it publishes `AppError` (:319), so a toast appears.
- WebView gate: `AuthGate` (src/lib/auth/AuthGate.tsx:99-113). `loaded && mode==="unconfigured"` → ConfigurationScreen (developer .env instructions); `!loaded || unknown` → spinner (HUD: `null`); `signed_out` → SignIn card (HUD: `HudSignInPrompt`, which opens the onboarding window at step 0). The store's default `mode` is `"unconfigured"` (auth-store.ts:52), and `load()` failure only sets `loaded:true` (:63-65).
- Clerk tokens are used only inside `auth/`. Outside it, `core.auth.*` appears only in commands/auth.rs, commands/data.rs:115 and app/mod.rs:348. Sign-in unlocks no server feature.
- The backend does not enforce the auth gate. Pipeline transitions all use `transition_soft` (audio/mod.rs:336,615; context/mod.rs:89; ai/mod.rs:305). The global shortcut ToggleListening (shortcuts/mod.rs:221-227) and the menu-bar Listen item (platform/mod.rs:223-229) call `audio.start(None)` in every auth state. `clear_session` (auth/mod.rs:574-603) does not stop audio; `data_reset_all` does (commands/data.rs:54-56).

### Onboarding
- `OnboardingFlow` (src/features/onboarding/OnboardingFlow.tsx:17-29 STEPS; `ready` starts true :34; last step :41-46 writes the flag, opens main, closes onboarding). Progress lives only in `useState(0)`, not persisted.
- SignIn: basics.tsx:33-38 `onReady(mode!=="browser" || state==="signed_in")`.
- ConnectAI: connect.tsx. It always inserts an enabled, keyless `gemini` provider at the front (:91-97). Saving the key → `secrets_set` (commands/settings.rs:38-43 → `refresh_provider_keys` → `settings.changed` with `hasApiKey`) → `applyProviderPresets(overwrite:false)` (:156) → `bootstrapProvider` (:157-159) → `testConnection` (:101-130). Readiness is `hasKey || otherProviderReady || accountReady || skipped` (:73), where `hasKey` means stored, not verified.
- Permissions: steps/permissions.tsx (4 kinds; `onReady` only on the last sub-screen :65-67) → permissionsStore.request/openSystemSettings (src/stores/permissionsStore.ts:24-37) → `permissions_request` → `PermissionManager::request` (permissions/mod.rs:90-114: Screen Recording in-process, mic/AX/speech via the helper) → `refresh()` → `PermissionsChanged` only if something changed (:141-154). The refresh loop runs every 30 s only while `hub.audio_active()` (:158-171). Nothing refreshes on window focus. Nothing in Rust consumes `PermissionsChanged` other than the forwarder.
- DefaultMode: setup.tsx:28 `modes.setDefault` (fire and forget). Shortcuts: setup.tsx:54-77 has its own copy of the KeybindsTab recorder, without the try/catch.
- Test steps: tests.tsx. Screen/mic errors appear only as toasts (:26,:72). TestAI picks `providers.find(id===models.default.providerId) ?? providers.find(enabled)` (:105-107). Account providers are not in `settings.ai.providers` (AITab.tsx:188-203), so that lookup falls back to the keyless gemini. ReadyStep (:169-179) is unconditional and hardcodes ⌘\ / ⌘↵.

### Settings → AI
- AITab.tsx. The default-provider select (:256-277; keyless options disabled; `switchDefaultProvider` :235-251 applies presets with `overwrite:true`). ProviderCard (ProviderCard.tsx:128-233: Edit, enable switch, SecretKeyField, "Use recommended models", Test connection; no Remove). ModelRoleRow (AITab.tsx:56-172: select marks only `(disabled)` :136; clearing the model input unassigns the role). SecretKeyField (SecretKeyField.tsx: Save/Replace/Cancel only).
- Backend: `settings_update` → `SettingsManager::update` (settings/mod.rs:57-68). `validate` only checks embedding dimensions (:200-208). `replace()`/`refresh_provider_keys()` call `has_sync` for every provider (:150-176). `secrets_delete` exists (commands/settings.rs:52-57, api.ts:191), but no UI calls it.
- Routing: `AiManager::select` → `router::select` (bluey-core/src/router.rs:61-139). `provider_usable = enabled && has_api_key` (:255-257); `fallback_chain` is role-only (:239-253). Failure → `config.no_model`, and the detailed `reason` is thrown away. Accounts: `provider_config` makes non-connected accounts keyless (bluey-core/src/accounts.rs:184-200). Disconnect unassigns roles (accounts/mod.rs:538-580).
- Error copy: present.ts `CODE_COPY` (:61-203), `describeError` (:219-275), recovery buttons (:278-344).

### Setup checks
- `app_run_setup_checks` → app/checks.rs:15-99. The AI check is `any(enabled && has_api_key) && models.default.is_some()` (:26-28, :56-69). It is only shown by the "Run setup checks" button in Settings → Permissions (PermissionsTab.tsx:34-43); onboarding never runs it.

## Capture, audio & transcription

### Traced chains (path:line)

**⌘↵ snapshot (screen + OCR + AX)**
`src/context/snapshot.ts:60-80` snapshotOptionsFor (inline:true, changeDetection:false, maxDimension from settings; `preferredDisplay:"active"` becomes `{type:"display"}` with no id, L53) → `api.context.buildSnapshot` → `src-tauri/src/commands/context.rs:10` context_build_snapshot → `src-tauri/src/context/mod.rs:81` build_snapshot_with → `tokio::join!(frontmost, accessibility, capture_and_ocr)` (L134-135) → `capture/mod.rs` capture() (L140-141 maps "active"/"main" to None) → helper `capture.display` → `ScreenCaptureService.swift:24` → `ShareableContent.display(withId:nil)` = `CGMainDisplayID()` → SCScreenshotManager → `finalize()` always writes the temp JPEG (TempFrames) and adds base64 when inline → Rust FrameCache (cap 8, eviction drops only the map entry, capture/mod.rs:31-62) → OCR `capture.ocr()` → helper `ocr.recognize` (OCRService.swift: VNRecognizeTextRequest, minimumTextHeight 0.008, on the downscaled frame). A capture error is propagated by `captured?` (context/mod.rs:136). An OCR error is tolerated (L127-129). An AX error becomes None.

**Listening, Apple route**
UI → `audio_start` → `audio/mod.rs:240` start() → `cloud_provider()` (L349-428) → `route_for` (L75-95; with no key the route is Apple plus an `stt_fallback` AudioError) → `helper_start_params` (L119-145: onDevice:true; locale omitted for "auto") → helper `AudioSession.start` (AudioSession.swift:109-165, locale default "en-US" at L150) → MicrophoneCapture (AVAudioEngine tap + converter) and SystemAudioCapture (SCStream audio, excludesCurrentProcessAudio) → PCMChunker/VAD → `deliver()` appends every chunk to SpeechTranscriber, whether or not it is speech → SpeechTranscriber.handleLocked (SpeechTranscriber.swift:164-214) emits transcript.partial/final → Rust `handle_helper_event` (audio/mod.rs:794-879) → `on_transcript` (L881) → TranscriptAssembler.ingest keyed by `start_ms` (helper.rs:507-517) → BlueyEvent TranscriptPartial/Final → `src/stores/initStores.ts:52-53` → transcriptStore (a single `partial` slot) → `features/hud/TranscriptStrip.tsx` / `transcript-strip.ts:30-57`. Finals also go to the ring (RING_CAPACITY), the DB when store_transcripts is on, and the classifier (`src/transcript/classifier.ts:199-205`, which is response-worthy only when speaker !== "You").

**Listening, cloud route (Gemini Live default)**
helper `audio.chunk{pcm16}` → `handle_helper_event` AudioChunk → `forward_pcm` (audio/mod.rs:433-500; opens a session per source on first chunk; `failed` sources are skipped forever at L448) → `GeminiLiveProvider::open` (gemini_live.rs:80-100, spawns a worker, 256-command buffer) → `Worker::run` (L447-541: connect → forward → decode_frame text+binary → Interim/Final → rotate at 9:30 or goAway → reconnect/backoff → `fail()` → TranscriptionEvent::Failed) → `on_stt_event` (audio/mod.rs:500-520) → `on_cloud_text` (utterance_start_ms timing) → same assembler. Foundry: `cloud_realtime.rs:184-265`, which follows the same pattern.

**Helper lifecycle**
`sidecar/mod.rs:236-287` on_terminated → fail_pending → backoff restart (500ms·2^n, MAX_RESTARTS, healthy reset) → `publish_status` → BlueyEvent::HelperStatus → consumed only by `src/stores/errorSurface.ts:24-27` (toast "Helper restarted") and app boot (`app/mod.rs:377`). AudioManager subscribes only to helper events (`audio/mod.rs:779`) and ignores `Ready`. CaptureManager's `observing` flag is never reset.

**Permissions**
`permissions/mod.rs:58-86` refresh (Rust preflight for Screen Recording/AX, helper for mic/speech); `spawn_refresh_loop` (L158-170) runs every 30 s only while audio is active and publishes PermissionsChanged. The only consumer is the TS permissionsStore (initStores.ts:39). No subsystem stops or re-routes on revocation.

**Smart observation**
settings side_effects.rs:53-58 / app/mod.rs:393-396 → capture.observe_start → helper ScreenObserver (160 px SCStream, dHash) → `screen.changed` → sidecar/mod.rs:211-217 publishes it on the bus, and nothing consumes it in Rust or TS.

## Native macOS, packaging & updates

### Bootstrap and window chain
- `src-tauri/src/app/mod.rs:58-109` `run()` registers the plugins (opener, deep-link, shell, global-shortcut, notification, os, process, dialog, fs, clipboard, updater, autostart with `MacosLauncher::LaunchAgent`, and nspanel on macOS). There is **no single-instance plugin**.
  - `on_window_event` hides every non-HUD window on close.
  - `RunEvent::ExitRequested{code: None}` becomes `prevent_exit`.
  - `RunEvent::Exit` runs `block_on(shutdown)`.
  - No other RunEvent (Reopen, Opened) is handled.
- `bootstrap()` (`app/mod.rs:112-341`) builds the managers, runs `events::spawn_forwarder`, calls `updates.start_background()`, and registers deep links (`app/mod.rs:309-322`, `on_open_url` + `get_current` → `handle_deep_links` → `auth.handle_callback_url`).
  - It then calls `panel.attach(onboarding_completed)` (`app/mod.rs:325`, which shows the HUD), `open_window("onboarding")` on first run, `build_tray`, and `permissions.spawn_refresh_loop()`.
  - Finally it spawns `finish_boot`.
- `finish_boot` (`app/mod.rs:356-409`) runs in this order: `shortcuts.apply_bindings`, `helper.ensure_running`, `permissions.refresh`, `set_autostart`, **then** `capture.set_protection(true)` when Privacy mode is on, then Smart observation.
- No `set_activation_policy` call exists. tao-0.35.3 (`app_delegate.rs:106`, `app_state.rs:284-285,455-468`) applies `NSApplicationActivationPolicyRegular` in `applicationDidFinishLaunching`, which overrides `LSUIElement=true` from `src-tauri/Info.plist`. Confirmed on the live app: `lsappinfo` shows pid 6547 `type="Foreground"`.

### HUD panel
- `overlay/mod.rs:31-40` sets the panel config: `can_become_key_window: true`, `is_floating_panel`, `hides_on_deactivate: false`.
- `attach()` (`overlay/mod.rs:112-170`) does:
  - `to_panel`
  - `set_level(Floating)` **unconditionally**
  - style mask borderless + nonactivating
  - collection behavior `can_join_all_spaces | full_screen_auxiliary | ignores_cycle`
  - `becomes_key_only_if_needed(true)`
  - initial rect from `work_areas()` (`overlay/mod.rs:560-577`, keyed by `monitor.name()`)
  - `watch_geometry`, which listens for Moved/Resized/ScaleFactorChanged/Focused, debounces 120 ms, then runs `sync_from_window`.
- `show()` means `orderFrontRegardless` (tauri-nspanel c9ec213 `panel.rs:242-244`). The panel is never made key.
- `apply_level()` (`overlay/mod.rs:490-513`) is only reached from `set_pinned` and `set_always_on_top`.

### Content protection
- The tray item "Toggle Privacy Mode", settings `side_effects.rs:34`, `capture_set_protection` and boot `finish_boot` all lead to `CaptureManager::set_protection` (`capture/mod.rs:397-406`). That calls `set_content_protected` (sharingType) on every `app.webview_windows()`, which are the three pre-created windows.
- Native NSMenu popups (`platform/hud_menu_macos.rs:95,190`), NSOpenPanel dialogs (`documents/mod.rs:292`, `commands/audio.rs:85`), the tray menu and the Dock icon are not covered.
- The helper excludes Bluey from its own ScreenCaptureKit captures (`ShareableContent.swift:34-38`; `ScreenCaptureService.swift:42`, `ScreenObserver.swift:62`, `SystemAudioCapture.swift:58`).

### Shortcuts
- At boot and on every settings change (`settings/side_effects.rs:15`), `ShortcutManager::apply_bindings` (`shortcuts/mod.rs:82-122`) runs `unregister_all`, then `on_shortcut` for every enabled binding. The plugin goes through global-hotkey-0.8.0 `RegisterEventHotKey(..., options 0)` (`platform_impl/macos/mod.rs:116-123`). A press runs `trigger()` (`shortcuts/mod.rs:199-277`), which publishes `shortcut.triggered` and runs the panel/audio actions.
- Defaults are in `bluey-core/src/shortcuts.rs:11-100` (⌘\, ⌘↵, ⌘⇧↵, ⌘⇧L, ⌘R, ⌘,, ⌘ arrows, ⌘⇧↑/↓).
- Conflict detection (`bluey-core/src/shortcuts.rs:269-300`) only checks other Bluey bindings and a short list of system shortcuts, and for a system clash it only logs a warning (`shortcuts/mod.rs:143`).

### Sidecars
- **Helper:** `HelperClient::spawn_once` (`sidecar/mod.rs:97-149`) runs `shell().sidecar("bluey-helper").env_clear().envs(base)`, waits up to 5 s for `helper.ready`, then sends `helper.version` (2 s default timeout, `sidecar/mod.rs:400-408`) and checks protocol major 1.
  - Crash handling is in `on_terminated` (`sidecar/mod.rs:234-282`): exponential restart, at most 5 attempts.
  - `shutdown()` sends `helper.shutdown` with a 750 ms timeout, then kills.
  - The Swift side handles `helper.version` inline on the serial read queue and constructs `SFSpeechRecognizer` there (`HelperApp.swift:121-155`, `Router.swift` inline branch). The helper exits on stdin EOF (`HelperApp.swift:48-50`).
- **Agent:** spawned per job (`agent/mod.rs:229-236`), killed in `AgentManager::shutdown` (`agent/mod.rs:479`). The Bun side keeps a running job alive after stdin EOF (`sidecars/agent/src/main.ts:151-157`).
- The shell plugin's own `RunEvent::Exit` hook only kills JS-spawned children (`tauri-plugin-shell-2.3.6/src/lib.rs:132-142`).

### Exit paths
- Tray Quit (`platform/mod.rs:245-247`) and `app_quit` (`commands/app.rs:66-68`) call `shutdown().await` then `exit(0)`.
- ⌘Q, Dock Quit, logout and System Settings "Quit & Reopen" go through NSApp `terminate:`, then tao `applicationWillTerminate` (`app_delegate.rs:131`), then `RunEvent::Exit`, then `shutdown`.
- Update relaunch: `updates_relaunch` is a **sync** command (`commands/updates.rs:28-31`), so it runs on the main thread. It calls `UpdatesManager::relaunch` (`updates/mod.rs:311-321`), which calls `app.restart()`. That runs `cleanup_before_exit` + `process::restart` (`tauri-2.11.5/src/app.rs:588-592`; `process.rs:74-128`: `Command::new(Contents/MacOS/<exe>).spawn(); exit(0)`). **`RunEvent::Exit` never fires, so `shutdown()` is skipped.**

### Updates
- `start_background` (`updates/mod.rs:352-372`): 30 s after launch, then every 6 h.
- `check()` queries the channel's feed via `updater_builder().endpoints()`. When `automatic` (the default) is on, it calls `install_pending` → `download_and_install` (minisign verify, then updater-2.11.0 `updater.rs:1324-1377`, which moves the bundle and falls back to an AppleScript admin prompt on PermissionDenied) → phase `Ready`. The HUD pill "Restart to update" then leads to the relaunch path above.
- Sidecars are resolved again on every spawn, so after install and before relaunch the **new** helper/agent binaries run under the **old** host.

### Build and release
- `scripts/release.sh:78-95` builds the helper (`build-helper.sh`: `swift build --arch` for both arches, ad-hoc + runtime + helper entitlements) and the agent (`build-agent.sh`: `bun build --compile` for bun-darwin-arm64/x64, ad-hoc) for **both** arches, then sets `APPLE_SIGNING_IDENTITY=-` when no credentials exist, then runs `tauri build --target $TARGET`. Tauri picks `binaries/<name>-$TARGET` via `externalBin` and re-signs sidecars with the app `entitlements.plist` (the installed helper carries allow-jit and audio-input).
- `nightly.yml:54-69` runs a macos-14 matrix over both targets with the gemini/lite backend, all ad-hoc. `release.yml` plus `verify_macos.py:52` reject ad-hoc for publication, but stable v0.1.2 was published by hand, unsigned.

### Temp frames
- The helper writes JPEGs to `~/Library/Caches/com.codewithabdul.bluey/frames` and sweeps files older than 1 h only at helper startup (`TempFrames.swift:30-45`, called from `HelperApp.swift:42`).
- The Rust `FrameCache` (capacity 8, `capture/mod.rs:35,55-63`) evicts from memory only. `discard_frame` (`capture/mod.rs:291-302`) is never called by the UI (only defined at `src/lib/tauri/api.ts:72`).
- `persist_snapshot` calls `SnapshotRepository::save_screen`, which stores `frame.path` (the temp file) as `image_path` when `store_screenshots` is on (`bluey-storage/src/repositories/snapshots.rs:36-37`).

## HUD & product UX

### Chains traced (read end-to-end unless noted)

**Typed ask / Assist / follow-up**
`HudInputRow.useHudInput.submit` (src/features/hud/HudInputRow.tsx:32-41; empty input → `onAssist`) → `HudPanel.submitTyped` / `assist` (src/features/hud/HudPanel.tsx:42-63; `captureScreen: !expanded && screenEnabled`, assist = trigger `shortcut_capture`) → `useAsk.ask` (src/features/hud/useAsk.ts:33-71; `chat.begin` bumps generation, callbacks guarded by generation) → `engine.ask` (src/ai/engine.ts:595-647; supersedes the in-flight ask through `gate.takeInflight` + `api.ai.cancel`) → `runPipeline` → `buildNativeSnapshot` (src/context/snapshot.ts:60-94; **includeScreen = captureScreen || trigger==="shortcut_capture" || requires(mode,"screen")**) → Rust `build_snapshot_with` (src-tauri/src/context/mod.rs:81-150; `transition_soft(CaptureStarted)`) → AI stream (src-tauri/src/ai/mod.rs:296-306 `ThinkingStarted` if the task is primary; 678-685 `publish_failed` → `AppEvent::Failed`) → deltas → `onDelta` (engine.ts:456-475) → `chatStore.applyDraft/complete/fail` (src/stores/chatStore.ts:116-137) → `ResponseThread`/`ResponseView`.

**Stop / Esc / New Chat**
Stop button (HudInputRow.tsx:142-151) or Esc (useHudShortcuts.ts:95-98 → HudPanel.tsx:65-68) → `useAsk.stop` (useAsk.ts:73-83): `markCancelled` (does NOT bump generation) + `currentHandle.cancel()`, where `currentHandle` is the last *ask* only. New Chat (button, ⌘R, ←) → `useAsk.newChat` (128-132) → `chatStore.newChat` + `app_dismiss_response` (src-tauri/src/commands/app.rs:48-54 → `ai.cancel_all()`).

**Live suggestion (PR #45)**
`transcript.final` → `engine.classify` → `question.detected` → `startProactiveLoop.onDetected` (src/stores/proactive.ts:198-207) → `prepareFor` (147-196): `canShowLive` (91-92) → `chat.begin(event.text, …, {phase:"thinking", suggestion})` → `engine.prepare(input, liveCallbacks)` (engine.ts:666-718; `isCancelled: () => false`, `onStreamHandle: () => {}`; no handle is returned). **The chain stops here for cancellation:** nothing in the HUD can cancel a prepare except `cancel_all` on New Chat, or a newer prepare making it stale.

**Audio / listening indicator**
Toolbar button (HudToolbar.tsx:65-72) and the `toggle_listening` event handler (useHudShortcuts.ts:49-76) decide from `useAppStore.status.audioActive` → `audio_start/stop` → `AudioManager.start` (src-tauri/src/audio/mod.rs:239-345; early return if `is_running`) → `hub.transition_soft(AudioStarted)` (336) → state machine (src-tauri/crates/bluey-core/src/state/mod.rs:192-210; `audio_toggle_allowed` 287-300 **excludes Error/Paused**) → `app.state` → appStore → pill (state-pill.ts:58), TranscriptStrip (TranscriptStrip.tsx:26), tray label (platform/mod.rs:115-121). Rust *also* toggles audio natively for the same shortcut (shortcuts/mod.rs:205-226). No frontend store subscribes to `audio.started/stopped/paused/resumed`.

**Error surfaces**
Per-request: `chatStore.fail` → `ErrorBanner` (components/ui/ErrorBanner.tsx; `presentError`, one action) with `onRetry = regenerate`. App-level: `AppEvent::Failed` → `status.state=error` → `StatePill` error variant (StatePill.tsx:79-111; action + `app_recover`). Global: `startErrorSurface` (stores/errorSurface.ts:21-28) → `showErrorToast` → `Toasts` (components/ui/Toast.tsx:45-66; fixed bottom-center inside the HUD WebView and not measured by `useAutoHeight`).

**Content protection**
HUD eye: `capture_get_protection` once on mount (HudToolbar.tsx:42-53) and `capture_set_protection` (commands/capture.rs:61-66 → capture/mod.rs:398-406; not persisted). Tray and Settings: `settings.update(privacy.displayMode)` → side_effects.rs:32-35 → `set_protection`. Boot: app/mod.rs:388-390.

**Shortcuts**
bluey-core/src/shortcuts.rs:22-93 defaults (all `enabled: true`) → `ShortcutManager.apply_bindings` (src-tauri/src/shortcuts/mod.rs:82-123, at boot app/mod.rs:370, always registered) → global-hotkey 0.8 → Carbon `RegisterEventHotKey` → `trigger()` (205-265) → `shortcut.triggered` / `panel.scroll` / `panel.newChat` → `useHudShortcuts`. `BlueyEvent::PanelFocusInput` is declared (bluey-core events/mod.rs:244-245,311) but never published; the HUD listens for it at HudInputRow.tsx:21.

**Focus**
NSPanel non-activating + `becomes_key_only_if_needed` (overlay/mod.rs:129-139). `show()` = `orderFrontRegardless` (tauri-nspanel c9ec213 src/panel.rs:242-244); `show_and_make_key` (409) is never used.

## Sessions, history & data lifecycle

### Traced chains (all verified unless marked)

**Session lifecycle**
- HUD menu: `SessionMenu.tsx:24-60` → `session-actions.ts:27-47` → `bluey.session.start/pause/resume/end` (`api.ts:144-147`) → `commands/sessions.rs:20-40` → `SessionManager::{start,pause,resume,end}` (`sessions/mod.rs:80-220`) → `SessionRepository` → `BlueyEvent::Session{Started,Paused,Resumed,Ended}` → `initStores.ts:46-50` → `useSessionStore.setActive`.
- Auto-session: `AudioManager::start` (`audio/mod.rs:315-319`): `if sessions.active().is_none() { sessions.start(); auto_session=true }`. `stop()` (L576-590) ends the session only when `auto_session` is set.
- Boot: `app/mod.rs:170-181` `SessionManager::load` → `SessionRepository::get_active` (`sessions.rs:212-226`, `status IN ('active','paused')`) → restored as active. Audio is not restarted, and `auto_session` starts false.
- Graceful quit: `RunEvent::Exit` → `shutdown()` (`app/mod.rs:104-106, 412-420`) → `audio.stop()` → the auto session ends.
- Update relaunch: `updates_relaunch` is a sync command (`commands/updates.rs:28-31`) → `updates/mod.rs:320 self.app.restart()`. Per tauri-2.11.5 `app.rs:582-592`, a restart on the main thread skips `ExitRequested`/`Exit`, so `shutdown()` never runs.
- History off: `end()` deletes the session right away (`sessions/mod.rs:211-218`).

**Transcript**
- Helper `audio.transcript` (times in ms since `audio.start`, `SpeechTranscriber.swift:10`) → `audio/mod.rs:531-544` → `commit_final` (L911-929) → pushed to the in-memory ring (cap 500, L53). The segment is saved only when `store_transcripts && session_id.is_some()` → `TranscriptRepository::upsert_partial` → FTS trigger (`0002_fts_sync.sql:28-46`).
- Context: `context/mod.rs:187-199` → `audio.recent(window)` (`audio/mod.rs:672-682`). The cutoff is measured from `ring.back().end_time`, with no session filter.
- History: `SessionDetail.tsx:44-53` → `transcript_list` → `AudioManager::list` (L685-716) → `TranscriptRepository::list` (`transcript.rs:166-199`, `ORDER BY start_time_ms DESC LIMIT n`, then reversed).

**Responses**
- `useAsk.ts:45-58` / `proactive.ts:155-176` pass `session: useSessionStore.active` → `engine.ts:419-428` (`baseResponse.sessionId = input.session?.id`, `modeId`) → `engine.ts:564-586`: when `!silent`, `api.responses.save` (errors swallowed) + `session.addEvent('response_generated')` → `commands/responses.rs:11-18` → `ResponseRepository::save` (`responses.rs:24-60`) → `responses_fts` trigger.
- Silent prepared answers (`engine.ts:664-719`) are cached in memory only. `takePrepared` → `useAsk.ts:89-108 showResponse` does not save.

**History UI / search / export**
- `SessionsTab.tsx:55-69` → `sessions_search` → `SessionManager::search` → `search_sessions` (`search.rs`): title/mode LIKE + `transcript_fts` + `responses_fts`, OR-ed sanitized terms (`fts.rs:19-40`), `take(50)`. With no text it falls back to `SessionRepository::list` (`DEFAULT_LIST_LIMIT=50`).
- Detail: `sessions_get` → `SessionManager::detail` (`sessions/mod.rs:235-254`).
- Export: `SessionDetail.tsx:236-243` → `data_export_session` (`data.rs:157-170`) → `SessionManager::export` → `bluey_core::session::export_markdown` (`session.rs:93-185`) → clipboard.
- Summary: `SessionDetail.tsx:245-268` → `getEngine().summarizeSession` (`engine.ts:824`) → `sessions/summary.ts:127-164` (task `summarization`; router `router.rs:186-198` sends it to the fast role) → `sessions_save_summary` → `SummaryRepository::save` (upsert, full JSON).

**Import**
- `session-import.ts:57-78` → `ai_transcribe_file` → `transcription/batch.rs:32-151`: transcribe first, then create the session or append with a `last_end_ms` offset; respects `store_transcripts`; adds a `recording_imported` event.

**Deletion / retention**
- Single: `sessions_delete` → `SessionManager::delete` (`sessions/mod.rs:256-270`) → `SessionRepository::delete` (`sessions.rs:339-359`, FK cascade) → frame files unlinked. No event is published and there is no vacuum.
- All: `delete_all` (`sessions.rs:363-381`, `DELETE FROM sessions`).
- Toggle-off retention: `settings/side_effects.rs:93-127` → `apply_retention` (`retention.rs:171-192`).
- Reset: `data_reset_all` (`data.rs:53-117`).
- Vacuum runs only at `data.rs:28,35,116`.

**Documents**
- `FilesDropzone.tsx:46/79` (used only by `ContextTab` with scope global and `ModeFilesDropzone` with scope mode) → `documents_add` → `DocumentsManager::add` (`documents/mod.rs:57-87`) → `add_document` (`index.rs:33-137`: parse → chunk → insert, or stored as `failed`) → `embed_document` → `mark_embedded(tag, dims)`.
- Re-embed: at boot (`app/mod.rs:405`) and on embedding setting changes (`side_effects.rs:66-80`) → `stale_embeddings` (`documents.rs:388-414`).
- Retrieval: `retrieve.rs:241-262` skips on vector length mismatch only.
- Mode delete: `modes/mod.rs:128-150` → `ModeRepository::delete` (`modes.rs:351-363`) leaves `documents` rows with `scope='mode'` behind.

**Migrations**
- `db.rs:19-33` embeds `0001`-`0004`; `run_migrations` (L108-143) runs each one in a transaction and records it in `schema_migrations`. All changes are additive.

## Research & deep-research sidecar

### Chain A — ask-time research routing (verified)
1. Ask → `createResponseEngine()` (src/stores/engine.ts:22, no `researchTimeoutMs` dep) → `engine.run` → `maybeResearch` (src/ai/engine.ts:210-241).
2. `decideResearch` with `OPTIMISTIC_AVAILABILITY` (research.ts:25), then `bluey.research.available()` → `research_available` (src-tauri/src/commands/research.rs:38, registered lib.rs) → `ResearchManager::availability` (src-tauri/src/research/mod.rs:125-134): `search`=Exa key, `scrape`=Firecrawl key, `deep_agent`=`AgentManager::available()` (agent/mod.rs:98-117 = binary next to exe + backend model credential only).
3. `degrade()` (research.ts:118-129): deep_agent chosen solely on `availability.deepAgent`.
4. `buildPublicQuery(instruction, snapshot)` (research.ts:195) runs on the PRE-enrichment snapshot (engine.ts:330 `await maybeResearch` vs engine.ts:333 `enrichSnapshot`).
5. `runResearch` (research.ts:301-309):
   - search / search_scrape → `research_search` / `research_scrape` → Rust Exa/Firecrawl clients (research/mod.rs:94-122, bluey-protocols firecrawl.rs). Top 3 scraped sequentially (research.ts:238-246). Full markdown concatenated into ONE context block (research.ts:213-224).
   - deep_agent → `runDeepAgent` (research.ts:256-297): listens on bus `research.event` for its jobId; 90 s timer (research.ts:70, 284-287) → `deepCancel` + resolve null; request always `tools: ["exa_search","firecrawl_scrape"]`, no `allowedDocumentIds`.
6. Result → `items.push({source:"document", ref:"research", priority via budget})` (engine.ts:349-356) → context budget (src/context/budget.ts:131-163 drops non-transcript/OCR items that do not fit whole) → provider → `mergeCitations` (engine.ts:250-266: research citations first, then unvalidated answer-model citations) → `ResponseView` Sources list (ResponseView.tsx:172-190) → `openExternal` (src/lib/utils/open-external.ts:4-8) → plugin-opener `open_url` (scope check → ForbiddenUrl, see finding).

### Chain B — deep agent lifecycle (verified)
`research_deep_start` (commands/research.rs:25) → `ResearchManager::deep_start` → `AgentManager::start` (agent/mod.rs ~195-265): `deep_research_enabled` gate, `binary_path()` (92-96), `env_clear` + `child_base_env` + `job_env` (one backend's creds + EXA/FIRECRAWL + passthrough lists mod.rs:40-54), Rust publishes `Started` itself, writes `research.run`.
Sidecar: `main.ts` JSONL loop → `agent.ts` `startResearchJob` → `buildHandlers` (agent.ts:401-495; missing EXA/FIRECRAWL key for a requested tool ⇒ whole job fails, agent.ts:778-781) → Claude: SDK `query()` with `tools:[]`, MCP `mcp__bluey__*` allow-list, `permissionMode` dontAsk, empty tmp cwd, `outputFormat` json_schema; Gemini: `runGemini` (gemini.ts:245-359) tool loop, keeps last turn for a no-tools JSON report turn (333-352), throws `max_turns_exceeded` if tools still pending (322-329).
Events: `research.progress/toolCall/textDelta/completed/failed` → Rust reader (agent/mod.rs ~270-350; unparseable lines ignored; `Terminated` with job still present ⇒ `failed agent_exited` 290-301) → bluey-protocols agent.rs:118-151 (citations re-IDed `cit_1..N`) → `research.event` → `initStores.ts:59` → `useResearchStore.apply` and the `runDeepAgent` listener.
Exit: `runSidecarProcess` → `flushStdout` (writes "" and waits for drain only if backpressured) → `process.exit` (main.ts:177-189).

### Chain C — cancellation (verified; broken edge)
HUD `Skip research` (ResponseThread.tsx:30-37) → `researchStore.skip` (researchStore.ts:81-92) → `research_deep_cancel` → `AgentManager::cancel` (agent/mod.rs:456-476): writes `research.cancel`, spawns a 2 s `CANCEL_GRACE` timer that REMOVES the job and kills the child without publishing anything. Sidecar `cancel()` (agent.ts:825-830) only aborts the controller; `failed{cancelled}` is emitted after the run unwinds. Tool handlers use `AbortSignal.timeout` only (tools/errors.ts:69; exa 20 s exa.ts:11, firecrawl 45 s firecrawl.ts:11), so an in-flight tool is not interrupted. Measured with a mock 4 s tool: cancel at 488 ms → `failed` at 4190 ms (/tmp/bluey-audit/research-sidecar/cancel-inflight.ts). Past 2 s, Rust has already removed the job, so `Terminated` (mod.rs:293) finds nothing and no final event is sent.
Ask cancel does not reach research: `checkAlive` runs only after `await maybeResearch` (engine.ts:330-331), and nothing but `researchStore.skip` or the 90 s timer calls `deepCancel`.

### Chain D — documents (dead in production)
`document_read` / `allowedDocumentIds`: sidecar DocumentBroker (tools/documents.ts) + Rust allow-list (agent/mod.rs:355-382, `None` ⇒ deny) exist and are tested. The only production `deepStart` caller (research.ts:289-295) never passes documents ⇒ no private document ever reaches the sidecar.

### Build/bundle
scripts/build-agent.sh builds bun-darwin-arm64 + bun-darwin-x64. Variant `lite` (Gemini only) is the default; `full` (embedded Claude CLI) only with RESEARCH_BACKEND=claude. release.sh:95-98 builds both arches and checks the outputs; nightly.yml matrix aarch64+x86_64 builds lite. tauri.conf.json:87-90 externalBin. Dev: `beforeDevCommand` → scripts/ensure-sidecars.sh (existence-only check).

## Performance & latency

### ⌘↵ fast path as traced (SHA 1a117a5)
1. **Shortcut**: `src-tauri/src/shortcuts/mod.rs:206-219` `trigger()` publishes `ShortcutTriggered{mono_ms}` first, then spawns `panel.show()` (not on the critical path). The bus forwarder (`src-tauri/src/events/mod.rs:56-73`) emits it to the WebView. `src/features/hud/useHudShortcuts.ts:59-70` calls `onCaptureAnalyze(triggeredAtMs)`, then `src/features/hud/useAsk.ts:33-71` (no awaits) calls `getEngine().ask`.
2. **Snapshot** (`src/ai/engine.ts:294` → `src/context/snapshot.ts:59-87` options: includeOcr = includeScreen, quality 0.8 hard-coded, maxDimension = settings (default 1600, `bluey-core/src/types/settings.rs:218`), capture target Display by default (`:212`), changeDetection false, inline true) → Rust `context::build_snapshot` `src-tauri/src/context/mod.rs:84-248`:
   - `tokio::join!(frontmost, accessibility, capture_and_ocr)` (`:91-135`). Inside `capture_and_ocr`, OCR runs **after** capture in the same future (`:101-133`). The join therefore waits for capture + OCR.
   - Capture: `capture/mod.rs:152-231` → helper `capture.display` → Swift `ScreenCaptureService.swift:24-57` (`ShareableContent.fetch` on every capture) → `finalize` `:233-293` (downscale, 9×8 dHash, JPEG encode, always an atomic temp-file write, base64 inline). `changed=true` unless change detection is on (`:246-256`).
   - OCR: `capture/mod.rs:306-351` reuses `last_ocr` only when `!frame_changed`, so it is never reused on ⌘↵. It then calls helper `ocr.recognize` by path (Vision level from settings, default Fast `settings.rs:216`). Timeouts: capture 3 s, OCR 5 s, AX 1 s (`sidecar/mod.rs:402-404`). The Swift router dispatches on a concurrent queue (`Protocol/Router.swift:22-25`), so frontmost/AX/capture really do overlap.
   - After the join, serially: `session_context` DB reads (`context/mod.rs:203`, `:255-270`), trim, trace stamps, `ContextUpdated` publish (image stripped).
3. **WebView**: `engine.ts:319` `retrieveRelevantContext` (awaited, after the snapshot; the query includes the OCR headline, `src/context/retrieval.ts:56-69`) → Rust `documents/mod.rs:139-163` (a network `ai.embed` when `embeddingsEnabled`, then SQLite FTS + a brute-force cosine scan over all embedded chunks, `bluey-storage/src/documents/retrieve.rs:60-120, 241-266`). Then `engine.ts:330` `maybeResearch` (only external-info asks; `research/mod.rs:125-134` does 2–3 serial Keychain reads). Then fuse/intent/budget/`PromptBuilder` (`engine.ts:345-409`, `src/ai/prompt-builder.ts:62-135`). The image is re-attached at `engine.ts:377-382`.
4. **ai_stream** (sync command, `src-tauri/src/commands/ai.rs:18-24`) → `AiManager::stream` → `start` (`ai/mod.rs:265-306`) → spawned `run_stream` → `drive_provider` (`:576-606`) → `adapter_for` (`:178-207`). API-key providers get `secrets.get` (spawn_blocking Keychain read, `secrets/mod.rs:159-165`) on every request. OAuth providers get `accounts.credential_for` (`accounts/mod.rs:768-791`), which keeps tokens in memory but writes the token set back to the Keychain every time. `rust.request_sent` is stamped after all of this (`ai/mod.rs:606`). The HTTP client is shared (`app/mod.rs:203-207`: connect_timeout + UA only; reqwest 0.13 native-tls = SecureTransport, TLS 1.2 max per `native-tls-0.2.18/src/imp/security_framework.rs:377-381`, pool idle 90 s).
5. **Streaming**: `ai/mod.rs:640-651` sends each Delta over the Channel and also mirrors it to the bus (no JS listener, so the cost is serialisation only). TS `src/ai/stream.ts:150-151` → `engine.ts:457-476` `onDraft` on every delta (for structured output it re-scans the whole accumulated text 2–3 times) → `useAsk.ts:63` `applyDraft` → `chatStore.ts:115-118` new `turns` array → `HudPanel.tsx:28` and `ResponseThread.tsx:112` both subscribe to `turns` → `turns.map(<Turn>)` (`:159-160`, not memoised) → `ResponseView.tsx:120-136` (`splitStreamingMarkdown`, `looksLikeStructuredJson`, `<ReactMarkdown remarkPlugins={[remarkGfm]}>`), a full re-parse for **every** turn → `ResponseThread.tsx:132-135` `scrollTo(scrollHeight)` forces layout on every delta. First paint is stamped with a single rAF (`src/ai/trace.ts:110-117`).
6. **Trace merge**: TS stamps are offsets from the snapshot reply (`engine.ts:423-437`). Rust merges them (`bluey_core::types::latency`), persists to `ai_requests.trace`, and emits `ai.trace` → `initStores.ts:62` → `devStore` → `AdvancedTab.tsx:55` `summarize` (nearest-rank p50/p95). The Settings WebView is pre-created and hidden (`tauri.conf.json:37-48`), so the overlay accumulates across the session. The bench (`scripts/bench-fastpath.ts`, `src-tauri/src/app/bench.rs:205-225`) mirrors the snapshot options, but skips retrieval and every WebView stage.

### Other chains
- **Helper lifecycle**: spawned at boot (`app/mod.rs:375` `ensure_running`); crash restart with backoff (`sidecar/mod.rs:246-280`).
- **Agent sidecar**: one Bun process per research job (`agent/mod.rs:1-5, 199-247`), by design (ADR 0004).
- **Gemini Live**: `transcription/gemini_live.rs:80-99` opens a worker; `:243-290` connect + setup (10 s timeouts); audio is queued in a 256-slot mpsc during connect (`:58, :115-124`); rotation at 9m30 with a 2 s drain. PCM chunks are 200 ms on every route (`audio/mod.rs:51, 141`).
- **Storage**: one `Mutex<Connection>` (WAL) for all reads and writes (`bluey-storage/src/db.rs:36-39`, `storage/mod.rs:80-88`).

## Command/event/settings surface parity

### Command chain
UI -> `bluey.*` in src/lib/tauri/api.ts (call/callNoArgs wrappers) -> getTransport().invoke(name, args) -> Tauri IPC -> `#[tauri::command]` in src-tauri/src/commands/** (registered in the generate_handler! list in src-tauri/src/lib.rs, 142 entries) -> manager on AppCore (audio/, capture/, ai/, sessions/, settings/, secrets/ …) -> helper (sidecar/mod.rs JSONL, methods decoded in crates/bluey-protocols/src/helper.rs) or provider -> BlueyEvent on EventBus (src-tauri/src/events/mod.rs:36) -> spawn_forwarder (events/mod.rs:55-72) `app.emit(event.tauri_event_name(), event.payload())` -> src/lib/tauri/events.ts eventBus -> stores (src/stores/initStores.ts, errorSurface.ts, proactive.ts).
- Mock path: src/lib/tauri/mock/mock-transport.ts implements the same 142 names over fixtures.ts. There is no shared validation or defaults with Rust.
- Tauri 2.11.5 `Listeners::emit_js_filter` (tauri-2.11.5/src/event/listener.rs:269-293) only evals in webviews that registered a JS listener, so events nobody listens to cost only serialization.

### Shortcut chain (F1)
global-shortcut -> shortcuts/mod.rs:207 `trigger()` publishes ShortcutTriggered, then spawns the native action. For ToggleListening (mod.rs:221-226) that is `ctx.audio.is_running() ? stop : start(None)`.
The same event reaches src/features/hud/useHudShortcuts.ts:59-77. The `toggle_listening` case calls `toggleListening()` (:49-57), which reads `useAppStore.status.audioActive` (stale, because audio.started has not arrived yet) and invokes `audio_start`.
AudioManager::start (audio/mod.rs:239-303) checks `is_running()` (:244), then awaits `cloud_provider()` (:254, a keychain read when Gemini Live is the provider), and only then sets Starting (:256-259). This check-then-act gap is the race window.

### Settings chain
UI tab -> `bluey.settings.update({patch})` -> settings/mod.rs:61 merge_json + `validate()` (:200, only embeddingDimensions) -> SettingsRepository::save -> side_effects.rs (retention, launch-at-login, panel) + `settings.changed` -> initStores.ts:37.
Load goes through bluey-storage repositories/settings.rs:90-108 `get_merged_or_default`: the stored JSON is merged over `Settings::default()`, and any serde error returns the full defaults.

### Cloud gate
src/ai/cloud-gate.ts:22/26 is read only by src/ai/engine.ts:273 (ask), :810, :825 (summary). Rust has no reader of `cloud_ai_enabled` (bluey-core types/settings.rs:305 is the definition only).
Rust egress paths without the gate:
- AudioManager::cloud_provider (audio/mod.rs:350-380, Gemini Live)
- ai_stream (commands/ai.rs)
- ai_transcribe_file (ai/mod.rs:750+)
- document embeddings (ai/mod.rs)

### Dead or unreached ends
- activeApp.changed: its only emitter, accessibility/mod.rs:84 `poll_active_app`, has no callers.
- panel.focusInput: listener at HudInputRow.tsx:21, but no BlueyEvent::PanelFocusInput is ever constructed and TS never emits it.
- ResponsePrepared variant: never constructed. The TS-local `response.prepared` comes from engine.ts:709.
- AppState Paused: only reachable through app_pause (commands/app.rs:18), which has no caller.
- AiCacheRepository::get/set: no production callers.
- bluey_core::budget: no callers.

### Protocols
- Helper: Rust calls 18 of the 22 methods Swift registers. Every capture.* method is called from capture/mod.rs:187-203; `helper.ping` is the only one never called (no liveness ping; sidecar/mod.rs only restarts on crash with backoff). All 10 Swift events are decoded in helper.rs:368-452.
- Agent: sidecar methods research.run/cancel/document.response (sidecars/agent/src/main.ts:134-146) match Rust agent/mod.rs:242,381,426,462. All 7 sidecar events (protocol.ts:110-118) are parsed in bluey-protocols/src/agent.rs:54-180.

## Stubs, swallowed errors & unfinished states

### Error-surface chain (works)
Rust `BlueyError{kind,code,message,recovery}` -> `BlueyEvent::AppError` / `AudioError` / `HelperStatus` -> event forwarder (src-tauri/src/events/mod.rs) -> `src/lib/tauri/event-bus.ts` -> `src/stores/errorSurface.ts:24-27` (`helper.status`: !running -> `showErrorToast(HELPER_STOPPED_ERROR)`; restarted -> `showToast("Helper restarted", 2000)`) -> `src/components/ui/toast-store.ts:105-108` (turns `presentError(...).action` into a toast action) -> `src/components/ui/Toast.tsx:24-27`. Toasts are mounted in the HUD, Settings and Onboarding windows. There is no `window.addEventListener("unhandledrejection")` anywhere in src/ (checked with rg).

### AI failure -> sticky Error (stops at the state machine)
`src-tauri/src/ai/mod.rs:678-685 publish_failed` -> `AiFailed` event plus `hub.transition_soft(AppEvent::Failed)` -> `bluey-core/src/state/mod.rs:250-257` (state=Error, resume_state=idle). The only exit is `AppEvent::Recovered` <- `commands/app.rs app_recover` <- `src/features/hud/StatePill.tsx:23-27,94,105` (user click only; rg finds no other `recover(` caller). The next ask: `context/mod.rs` CaptureStarted, `ai/mod.rs:305` ThinkingStarted and `:522` ResponseReady are all `transition_soft`. `state/mod.rs:213-236` requires `is_idle()` (Ready|Listening, `types/app_state.rs:21-23`) or Analyzing/ResponseReady, so each is rejected, and `src-tauri/src/state/mod.rs:60-71` only logs it at debug level. `audio_toggle_allowed` (`state/mod.rs:291-300`) excludes Error, so AudioStarted is rejected and `audio_active` stays false. `src/features/hud/state-pill.ts:42` gives `state==="error"` top priority, so the pill keeps showing the old error.

### Helper crash -> restart (process recovers, sessions do not)
`sidecar/mod.rs:196-199` Terminated -> `on_terminated:236-287`: fail_pending, publish_status(running=false, restarted=true), backoff respawn via `spawn_once`, publish_status(true, restarted=true). Consumers: the audio listener (`audio/mod.rs:773-792`) only handles helper *events*. A dead helper never sends `AudioStopped`, so `mark_stopped` never runs and the status stays Running (`is_running :207-211`). `start()` returns early when running (`:244-246`). `CaptureManager.observing` is set at `capture/mod.rs:365` and never reset. `observe_start` is only called from `settings/side_effects.rs:57`, `app/mod.rs:396` and the command. Nothing re-arms either after a restart.

### Bootstrap
`app/mod.rs:78-95`: `.setup(|app| bootstrap(app).map_err(... tracing::error!))` then `.build(generate_context!()).expect("error while building the Bluey application")`. Fallible bootstrap steps: AppPaths::resolve, Storage::open + run_migrations, SettingsManager::load, window/panel attach, tray, `hub.transition(BootCompleted)?` (`app/mod.rs:325-340`). `Cargo.toml:123-128` sets `panic = "abort"`, `strip = true`, and there is no `std::panic::set_hook` anywhere. `tauri_plugin_dialog` is already initialized (`app/mod.rs:67`).

### Settings writes and side effects
UI -> `settingsStore.update` (`src/stores/settingsStore.ts:33-44`, toasts on failure) -> `settings_update` -> `SettingsManager::update` persists the full blob (`settings/mod.rs:57-69`) -> `settings/side_effects.rs:12-115 apply()`. Every side-effect error is `let _` or `tracing::warn!`: `apply_bindings` (:15), opacity/width (:22-26), content protection (:34), autostart (:40), `observe_start` Smart (:53-58), re-embed (:70-78), retention sweep (:99-112). None return to the UI.
Reads: `bluey-storage/src/repositories/settings.rs:91-109 get_merged_or_default`. Any deserialize error returns `T::default()` with only a warn log. The accounts list uses the same pattern (`accounts/mod.rs:124-127 .ok().unwrap_or_default()`).

### Shortcuts
`shortcuts/mod.rs:81-123 apply_bindings` collects `failed` and publishes only `BlueyEvent::DevLog`, stores `self.failed`, and returns `Ok(())`. `self.failed` is read only by `check_conflict` (`:166-187`), which runs from KeybindsTab/onboarding while a new binding is being recorded (the recorded binding's own id is ignored). The boot path (`app/mod.rs:~373`) only logs a warning.

### Subscription token refresh (hot path)
`ai/mod.rs:185 adapter_for` -> `accounts/mod.rs:768-792 credential_for` -> `TokenCache::fresh` (refresh under a single-flight lock) -> on every `Ok` with `expires_at`: `let _ = self.set_account(updated)` and `let _ = self.secrets.set(tokens_key, raw)`. `SecretsStore::set` (`secrets/mod.rs:136-145,172-178`) is an unconditional keychain `set_password` on the blocking pool, awaited before the tokens are returned.

### Recovery actions
`src/lib/errors/present.ts:280-340` builds actions that return raw IPC promises (`permissions.openSettings`, `accounts.connect`, `dev.restartHelper`, ...). Callers: `Toast.tsx:24-27` (`dismiss(); void toast.action?.run()`), `src/components/ui/ErrorBanner.tsx:34` (`void presented.action?.()`), `StatePill.tsx:92-95` (`await action(); await recover()`, so a throw skips `recover`).

### Research (best effort)
`src/ai/engine.ts:211-240 maybeResearch`: `available().catch(() => all false)`. `src/ai/research.ts:233-236` (search catch -> null), `:279-280` (deep `failed` -> null, error text dropped), `:284-286` (timeout -> null), `:295` (deepStart reject -> null). `researchStore.ts:74-76` clears activity on failure. The sidecar (`sidecars/agent/src/agent.ts:758-822`) and Rust agent manager (`agent/mod.rs:288-300,418-440`) do report typed failures; the UI drops them.

## Testing quality & verification debt

## Test layers and where each chain stops

**Frontend (vitest, jsdom, ubuntu CI)**
- `vitest.config.ts:12-18`: env jsdom; include `src/**` + `tests/**`; `sidecars` excluded but `tests/sidecar/*.test.ts` included, so agent sidecar code runs in-process under Node, not under the compiled Bun binary.
- UI suites (`tests/ui/*`, 32 files) run through `setupMockApp` → `MockTransport` (`src/lib/tauri/mock/mock-transport.ts`). The chain stops at the mock: `TauriTransport` (`src/lib/tauri/tauri-transport.ts:29-50`, the only code that calls `@tauri-apps/api` invoke/listen/Channel) has **zero tests**. No test uses `mockIPC`.
- `bootstrap.ts:57-64` picks Tauri vs mock by `"__TAURI_INTERNALS__" in window` (`transport.ts:44-46`).
- Integration (`tests/integration/*`) uses `tests/fixtures/helpers/fake-transport.ts` plus real engine modules. `command-surface.test.ts` reads `src-tauri/src/lib.rs` `generate_handler!` and `bluey-core/src/events/{mod.rs,tests.rs}` for name parity (both directions). This is the one real TS⇄Rust boundary test.

**Rust**
- Crates (host, ubuntu CI via `scripts/check-rust.sh`): core 110, protocols 205, storage 72, oauth 19, fingerprints 11 tests.
- App crate: `cargo test --features dev-tools` runs only on macos-14 arm64 (`ci.yml:93`), 63 tests (CI log 2026-09-12 18:55).
- Per-module test counts: accounts 10 / 3,348 LOC, ai 10 / 4,198, transcription 7, app 5, audio 4, auth 4.
- **Zero tests:** `sidecar/` (helper supervisor: `env_clear` spawn, 5 s ready wait, version handshake, restart backoff `sidecar/mod.rs:98-150,246-280`, per-method timeouts `:400-410`), `updates/`, `overlay/`, `sessions/`, `settings/`, `state/`, `research/`, `modes/`, `accessibility/`, `storage/`.
- All app tests are debug + dev-tools, so release-only branches never run: `updates/mod.rs:97-99` `supported() = !debug_assertions`, `auth/mod.rs:700` redirect style, `secrets/mod.rs:201`.

**Swift**
- `src-tauri/swift/BlueyHelper/Tests/*` has 6 XCTest files (AXRoleFilter, DHash, Envelope, OCRSorter, PCMChunker, VAD), run by `scripts/test-helper.sh`.
- No workflow calls it. Tests last changed 2026-09-07 (cd997e2); Sources changed 2026-09-11 (d08b65a).
- `tests/native/requests/*.jsonl` plus README is a manual harness only.

**Release pipeline**
- `scripts/release.sh:85-87` runs lint, `bun run test` and `check-rust.sh` (host crates only; no app-crate tests, no Swift tests).
- `verify_macos.py` (lipo arch check of main/helper/agent at `:92-97`, mounted DMG, updater archive) runs only when `PUBLISH_RELEASE=true` (`release.sh:121-127`).
- Ad-hoc dev/nightly builds only check that dmg/tar.gz/sig exist (`:113-116`).
- `nightly.yml:27-48` plan compares main to the last nightly commit (no CI-status check). The build step is `release.sh` (`:96`); publish (`:112`) replaces the rolling feed, and the Nightly channel auto-installs it (`docs/UPDATES.md:4-5,56`).
- `gh api branches/main`: `protected=false`, no required checks.

**Wire contracts (static checks done this audit)**
- Command args: scripted comparison of all 142 `#[tauri::command]` signatures vs `commands.ts` arg keys. All match (`onChunk` is a Channel; `r#type`→`type`). No automated test guards arg names.
- bluey-core structs vs `src/lib/types/*.ts` (heuristic field and optionality diff): no drift found.
- Error codes in `present.ts`: every code has a producer (Rust uses full prefixed codes, e.g. `updates/mod.rs:224,231`; TS-local ones in `answers.ts:23`, `cloud-gate.ts:9`, `proactive.ts:53`).
- Agent protocol names (`research.run/cancel/started/progress/textDelta/toolCall/completed/failed`) match between `bluey-protocols/agent.rs`, `src-tauri/src/agent` and `sidecars/agent/src`.
- Helper `helper.version` keys match (`HelperApp.swift:121-154` vs `helper.rs:236-250`). Swift and Rust each hard-code their own JSON; the only shared golden fixture is `tests/fixtures/native-hud-menu.json`.
- Event producers: `panel.focusInput` is declared (`events.ts:110`) and listened to (`HudInputRow.tsx:21`) but emitted nowhere.

**Mock vs real divergences (verified)**
- `mergeSettings` is a one-level spread (`mock-transport.ts:2213-2226`); Rust uses a recursive `merge_json`, `validate()` and shortcut reconcile (`bluey-core types/settings.rs:478-505`, `settings/mod.rs:57-67,200-207`).
- Mock `secrets_set` accepts any key (`:2002`); Rust `validate_webview_key` rejects non-allowlisted keys (`commands/settings.rs:38-41`).
- Mock dev simulation emits `transcript.final` plus a pre-built `question.detected` (`:851-862`). In production, detection comes only from TS `engine.classify` (`engine.ts:818`). PR #45's 'verification' used this bypass plus `ProactiveFakeEngine`.
- Updates mock always finds 0.2.0 (`UPDATES.md:63`).

**Machine and runtime facts observed**
- `src-tauri/binaries` holds only x86_64 helper and agent, dated Sep 7 15:01. That predates helper change d08b65a and agent changes 562f036 and a021f86. `ensure-sidecars.sh:18-28` only checks that the files exist.
- `/Applications/Bluey.app` 0.1.2: thin x86_64, flags=adhoc,runtime, TeamIdentifier not set, `codesign -d -r-` shows `designated => cdhash H"5ac26453…"` (helper `cdhash H"f71b4c37…"`).
- A prior attempt probed the stale x86_64 agent binary: an unknown method returned `{"error":{"code":"unknown_method","kind":"sidecar"}}` in 1.6 s.
- Classifier probe (bun, repo modules): unpunctuated ASR-like text is detected with `requiresResponse=true` in interview mode, and `false` in the default general mode (`classifier.ts:204-206`).

## Prior research & documentation drift

### Traced chains (current main 1a117a5)

**Cloud AI privacy switch**
- UI: `src/features/settings/tabs/PrivacyTab.tsx:262-270` ("Allow sending context to your configured cloud providers.") → `settings_update` → `bluey-core/src/types/settings.rs:305,318` (`cloud_ai_enabled`, default true).
- TS enforcement only: `src/ai/cloud-gate.ts:22-26` → `src/ai/engine.ts:273` (runPipeline), `:810` (classify model refine), `:825` (summarizeSession).
- Rust egress with NO gate (rg `cloud_ai_enabled` in src-tauri/src finds 0 hits):
  - `audio_start` → `src-tauri/src/audio/mod.rs:350-390` `cloud_provider()` → `TranscriptionProviderKind::GeminiLive` (the default, settings.rs:191) → `GeminiLiveProvider::new(key)` (WSS to Google).
  - `documents_add` → `src-tauri/src/documents/mod.rs:207` `self.ai.embed(..)` → `ai/mod.rs:789-792` checks only `embeddings_enabled`.
  - `session-import.ts:64` → `ai_transcribe_file` (`commands/ai.rs:105-108`) → batch transcription upload.

**Smart observation**
- `ScreenTab.tsx:62-70` → `settings.screen.observation=smart` → `app/mod.rs:393-396` / `settings/side_effects.rs:50-57` `capture.observe_start` → Swift `ScreenObserver` (SCStream) → `HelperEvent::ScreenChanged` → `sidecar/mod.rs:211-217` publishes `BlueyEvent::ScreenChanged` → `events.ts:59,143` (typed only). **Chain stops here:** no subscriber in src/stores, src/ai, CaptureManager or ContextManager (the only Rust bus subscriber, platform/mod.rs:200, handles AppState).

**Capture target "region"**
- `ScreenTab.tsx:53` option → `src/context/snapshot.ts:46-56` maps `region` to `{type:"display"}`; Rust `capture/mod.rs:138-139` "A region preference without a stored rect degrades to display". `ScreenSettings` (settings.rs:198-207) has no rect field and there is no picker UI, so the chain stops at the setting.

**Live detections**
- `transcript.final` → `src/stores/proactive.ts:210-222` `getEngine().classify(..)` (the return value is ignored) → `engine.ts:818` emits `question.detected` only when `requiresResponse`. decision/action_item/topic_change events from `src/transcript/classifier.ts` are dropped; `sessions_add_event` exists but is never called for them.

**Permissions refresh**
- `permissions/mod.rs:23-24,155-170`: a 30 s loop that runs only while `hub.audio_active()`. `initStores.ts:76` does a one-time get. `permissions.changed` is mirrored into a store (`initStores.ts:39`) and has no Rust consumer. `app/mod.rs:85-93` `on_window_event` handles only CloseRequested. There is no focus/visibility refresh and no restart prompt.

**Command capabilities**
- `src-tauri/build.rs:40` `tauri_build::build()` has no `AppManifest::commands`. `src-tauri/capabilities/{main,settings,onboarding}.json` list only core/plugin permissions, so every registered app command is callable from every window (Tauri v2 default).

**Research keys**
- `agent/mod.rs:148-153` reads EXA/FIRECRAWL only from the Keychain; `:233` `env_clear()`. `research/mod.rs:66,99` also reads only the Keychain. The env import (`app/env_import.rs` via `bluey_core::presets::plan_env_import`) covers only GEMINI/GOOGLE/AZURE_FOUNDRY/ANTHROPIC/OPENAI keys. Result: `.env.example:99-100` EXA_API_KEY/FIRECRAWL_API_KEY are silently ignored.

**Dev sidecars**
- `tauri.conf.json:7` beforeDevCommand → `scripts/ensure-sidecars.sh:18-28` checks only `-x` existence. `src-tauri/binaries/*-x86_64-apple-darwin` are dated Sep 7 14:59/15:01, before 562f036 (Gemini agent, 09-08) and d08b65a (Swift OCR fix, 09-11).

**Fast path (ADR 0010 PR 4b/5)**
- `context/mod.rs:91-136` `tokio::join!` awaits `capture_and_ocr` (OCR stays on the critical path). None of `context.enriched`, `fn warm`, mediaResolution, `RetrievalStrategy::Fast` (types/documents.rs:167-172 has Auto/Keyword/Semantic) or "Think deeper" exist anywhere in src/, src-tauri/src or the crates.

**Content protection**
- Settings `display_mode=Privacy` → `app/mod.rs:388-391` / capture `set_protection` → Tauri `set_content_protected` (NSWindow.sharingType=.none). The user-facing note is `capture/mod.rs:27-29`; the HUD tooltip is `HudToolbar.tsx:102`.
