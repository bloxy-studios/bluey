# Audio & Transcription

```
 Microphone (AVAudioEngine tap) ─┐                       ┌─► transcript.partial/final (Apple Speech, on device)
                                 ├─► resample 16 kHz mono ┼─► VAD ─► audio.chunk (isSpeech, rms[, pcm16])
 System audio (SCStream audio) ──┘     PCM16, 200 ms      └─► audio.level
                                                                    │
                                        Rust AudioManager ◄─────────┘
                                                │
        TranscriptionProvider (gemini_live default | apple | cloud_realtime | mock) ─► TranscriptSegment
                                                │
                              ring buffer + SQLite (if enabled) + event bus ─► classifier / HUD
```

## Capture

- **Microphone**: `AVAudioEngine` input tap; device selection by CoreAudio UID; automatic
  re-routing when the default input changes or a device disconnects (Bluetooth headsets,
  unplugging). Emits `audio.deviceChanged`.
- **System audio**: `SCStream` with `capturesAudio = true` and `excludesCurrentProcessAudio`,
  filtered to exclude Bluey — available when Screen Recording is granted (macOS 13+). Captured
  as a **separate channel** from the microphone, which is what makes "You" vs. "other party"
  labelling reliable.
- **Format**: both sources are resampled to 16 kHz mono PCM16 and chunked (200 ms).
- **VAD**: energy-based with adaptive noise floor and ~300 ms hangover; sensitivity low/medium/
  high. Non-speech chunks are dropped before on-device transcription (cheaper, fewer
  hallucinations); on the cloud route every chunk is forwarded so the server VAD sees the silence.
- **Retention**: raw audio is **never** written to disk. The `until_session_end`/`custom`
  values of `storeRawAudio` are accepted in settings but not implemented — nothing is retained
  in any mode, and Settings → Privacy shows raw audio as "Never kept" with no retention choice.

## Transcription

Two routes inside the Rust `AudioManager`: **Apple** (the helper transcribes on device and
emits `transcript.partial/final`) and **PCM** (the helper emits `audio.chunk { pcm16 }` and a
`TranscriptionProvider` — one session per source, so speaker labels still come from the
channel — turns it into the same segments). A cloud provider that cannot run (no key, no model,
developer-only mock) falls back to Apple and publishes a non-fatal `audio.error{stt_fallback}`
with the reason.

- **Gemini Live (default)** — `gemini-3.5-transcribe-live` over the Gemini Live API WebSocket
  (`src-tauri/src/transcription/gemini_live.rs`, frames from `bluey_protocols::gemini`):
  `setup` → `setupComplete`, `realtimeInput.audio` (PCM16 16 kHz mono), `audioStreamEnd` after
  500 ms of silence so utterances finalize promptly, `interimInputTranscription` → partials,
  `inputTranscription` → finals (with the detected language). The service answers on **binary**
  WebSocket frames carrying UTF-8 JSON (the browser SDK reads a `Blob`), so every frame is
  decoded whatever its opcode — a text-only reader never sees `setupComplete` and times out
  during setup (`network.timeout`) before any audio is sent. The service caps a session at
  ten minutes, so a replacement socket is opened at 9 min 30 s or on `goAway`; the old socket
  drains for two seconds and a final that repeats the previous one within two seconds is
  dropped — dedupe is armed only around a rotation, so a genuinely repeated short answer is kept
  otherwise. The Google AI Studio key is the same Keychain entry the chat provider uses
  (`provider:gemini:api_key`); it appears only in the WebSocket URL query and is redacted from
  every log line. A key refused at the WebSocket handshake (HTTP 401/403) or in-band
  (`UNAUTHENTICATED`, `PERMISSION_DENIED`) ends the session with `config.api_key_invalid`;
  `INVALID_ARGUMENT`/`NOT_FOUND` → `config.model_not_found`. A lost connection is re-opened
  with exponential backoff (0.5 s doubling, capped at 8 s) until it succeeds, listening stops,
  or the service rejects the configuration; sends time out after 5 s and a watchdog reconnects
  when speech went out but nothing came back for 15 s (a half-open socket). Five server
  closes/errors without transcript progress end the provider session. Audio never
  back-pressures capture: a chunk that does not fit the worker's buffer is dropped. Live uses
  the service's SMART mode (imported recordings are verbatim). Settings → Audio → *Gemini Live
  (cloud)*; the Live model follows the transcription role when it is a `*-transcribe-live`
  model.
- **Apple** — `SFSpeechRecognizer` per source in the transcription language (the Mac's own
  locale for *auto*, `en-US` when that has no recognizer) with on-device recognition when the
  locale supports it (`supportsOnDeviceRecognition`) and automatic punctuation (macOS 13+).
  When the locale has no on-device model the recognizer runs on Apple's servers: `audio.started`
  reports `speech { locale, onDevice }` (also `AudioStatus.speechLocale/speechOnDevice`) and
  Bluey shows an `audio.speech_server` notice instead of switching silently. Partial results
  stream; an utterance is committed as a final when the recognizer resets after a pause, and
  requests are rotated every ~55 s to respect the framework's one-minute limit — only the
  current request can rotate or restart, a retired request's late callbacks are ignored. Every
  partial and final carries the helper's `utteranceId` (request generation + counter), so a
  partial and its final are one segment. Works offline, no API key.
- **Cloud realtime** — WebSocket chosen from the transcription-role model:
  MAI-Transcribe-1.5 (`MAI-Transcribe-1.5` → Voice Live `mai-transcribe`) uses Foundry Voice Live
  (`session.update` with `input_audio_transcription.model`, Azure semantic VAD,
  `create_response: false`; PCM16 @ 16 kHz); deltas are accumulated per utterance before they
  are shown as a partial. OpenAI STT deployments (`gpt-4o-mini-transcribe`, …) would need
  `/openai/v1/realtime?intent=transcription` at 24 kHz, which the helper does not produce —
  such an assignment is refused up front and the session falls back to Apple with
  `audio.error{stt_fallback}`. Selected in Settings → Audio; the model is Settings → Models →
  Transcription.
- **Mock** — fixture-driven for tests and developer mode.

`TranscriptSegment { speaker?, speakerConfidence?, source, text, startTime, endTime,
confidence?, finalized }` — times are ms on the session's timeline: a run that attaches to a
session which already has segments starts after that session's last segment. Partials and finals
carry a producer `utteranceId`; the transcript store keeps one partial per source.

The ring of recent finals behind the context snapshot is scoped: while listening it holds the
current run's finals (each `audio.start` begins a new run), otherwise the active session's; the
recency window is measured from that scope's newest segment, and deleting a session purges its
entries. An ask in another session never sees an earlier conversation.

### Importing recordings

`ai_transcribe_file { path, diarization, wordTimestamps, language?, sessionId? }` (Settings → Sessions →
*Import recording…*, or *Add recording* on a session) transcribes a whole file with
`gemini-3.5-transcribe` — inline up to 14 MB, Files API resumable upload above that, deleted right after
— and files the result as finalized segments (`source: system`, `speaker: spk_n`) plus a
`recording_imported` timeline event; a recording added to an existing session starts after that
session's last segment. Unsupported containers (M4A/MP4) and files above 512 MB are refused before
anything is uploaded; the service's caps — 30 minutes with diarization/word timestamps, one hour
without — are enforced server-side and surface as `ai.invalid_request`. See `AI_ARCHITECTURE.md`
› *Batch*.

## Speaker identification

Labels come from the channel: `microphone → "You"` (0.95) and `system → "Interviewer" /
"Customer" / "Candidate" / "Speaker"` depending on the mode (0.6). When a cloud provider
supplies diarization, additional remote speakers become "Speaker 2/3" with the provider's
confidence. Bluey never presents speaker identity as certain.

## Events

`audio.started/stopped/paused/resumed`, `audio.level`, `audio.chunk`, `audio.deviceChanged`,
`audio.error`, `transcript.partial`, `transcript.final`, `transcript.cleared`; every final
segment is fed to the classifier (`question.detected`).

## Failure handling

Permission revoked → session stops with `BlueyError{kind: permission}` and a repair flow. Device
lost → automatic re-route, else `audio.error{device_lost}`. A cloud provider that cannot start
(no key, unsupported model, mock outside developer mode) falls back to Apple Speech with a
non-fatal `audio.error{stt_fallback}` naming the reason. Mid-session, a connection outage is
announced once as `audio.error{stt_degraded}` (also `AudioStatus.error`, cleared when transcripts
flow again) while the provider reconnects (Gemini Live and Voice Live share the policy above);
a provider session that gives up is re-opened on the next chunk after a 10 s cool-down, and a
configuration error (key rejected, model not found, unsupported) moves listening to Apple
Speech with `stt_fallback`. The other source keeps going throughout. Apple Speech itself unavailable → `speech_unavailable`.
`stt_fallback` is announced once per run as an info notice, not an error. With Privacy → Cloud AI
off a cloud provider is never used: listening runs on Apple Speech (`stt_fallback`), and turning
the switch off mid-run moves a live cloud session onto Apple Speech.

Starting is single-flight: a second `audio.start` while one is starting or running is not an
error. If the helper does not confirm `audio.start` in time Bluey sends a best-effort
`audio.stop`. If the helper exits while listening the status leaves `running`
(`sidecar.helper_exited`); when the supervisor restarts it, the run is re-issued once with the same
config and session (an info notice shows meanwhile). A helper-initiated stop, or a run that cannot
resume, ends the session listening auto-started. A microphone that drops out stays wanted: the
helper retries on the next device or configuration change and re-sends `audio.started` with the
current `microphone` flag.
