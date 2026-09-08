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
  high. Non-speech chunks are dropped before transcription (cheaper, fewer hallucinations).
- **Retention**: raw audio is **never** written to disk by default (`storeRawAudio: never`).
  `until_session_end`/`custom` keep chunks in memory/temp only as configured and purge them.

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
  `inputTranscription` → finals (with the detected language). The service caps a session at
  ten minutes, so a replacement socket is opened at 9 min 30 s or on `goAway`; the old socket
  drains for two seconds and a final that repeats the previous one within two seconds is
  dropped. The Google AI Studio key is the same Keychain entry the chat provider uses
  (`provider:gemini:api_key`); it appears only in the WebSocket URL query and is redacted from
  every log line. Configuration errors (`UNAUTHENTICATED`, `PERMISSION_DENIED`, `NOT_FOUND`) end
  the session with a `config.*` error and its recovery; transport errors reconnect (three
  attempts). Settings → Audio → *Gemini Live (cloud)*; the Live model follows the transcription
  role when it is a `*-live` model.
- **Apple** — `SFSpeechRecognizer` per source with on-device recognition when the
  locale supports it (`supportsOnDeviceRecognition`); partial results stream; requests are
  rotated every ~55 s to respect the framework's one-minute limit. Works offline, no API key.
- **Cloud realtime** — WebSocket chosen from the transcription-role model:
  MAI-Transcribe-1.5 (`MAI-Transcribe-1.5` → Voice Live `mai-transcribe`) uses Foundry Voice Live
  (`session.update` with `input_audio_transcription.model`, Azure semantic VAD,
  `create_response: false`; PCM16 @ 16 kHz). OpenAI STT deployments
  (`gpt-4o-mini-transcribe`, …) use `/openai/v1/realtime?intent=transcription`
  (PCM16 @ 24 kHz). Both send `input_audio_buffer.append` and receive
  `conversation.item.input_audio_transcription.delta/completed`. Selected in
  Settings → Audio; the model is Settings → Models → Transcription.
- **Mock** — fixture-driven for tests and developer mode.

`TranscriptSegment { speaker?, speakerConfidence?, source, text, startTime, endTime,
confidence?, finalized }` — times are ms since the audio session started.

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
lost → automatic re-route, else `audio.error{device_lost}`. Recognizer unavailable → falls back
to the configured cloud provider or reports `speech_unavailable`.
