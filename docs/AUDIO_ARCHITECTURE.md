# Audio & Transcription

```
 Microphone (AVAudioEngine tap) ─┐                       ┌─► transcript.partial/final (Apple Speech, on device)
                                 ├─► resample 16 kHz mono ┼─► VAD ─► audio.chunk (isSpeech, rms[, pcm16])
 System audio (SCStream audio) ──┘     PCM16, 200 ms      └─► audio.level
                                                                    │
                                        Rust AudioManager ◄─────────┘
                                                │
                     TranscriptionProvider (apple | cloud_realtime | mock) ─► TranscriptSegment
                                                │
                              ring buffer + SQLite (if enabled) + event bus ─► classifier / HUD
```

## Capture
* **Microphone**: `AVAudioEngine` input tap; device selection by CoreAudio UID; automatic
  re-routing when the default input changes or a device disconnects (Bluetooth headsets,
  unplugging). Emits `audio.deviceChanged`.
* **System audio**: `SCStream` with `capturesAudio = true` and `excludesCurrentProcessAudio`,
  filtered to exclude Bluey — available when Screen Recording is granted (macOS 13+). Captured
  as a **separate channel** from the microphone, which is what makes "You" vs. "other party"
  labelling reliable.
* **Format**: both sources are resampled to 16 kHz mono PCM16 and chunked (200 ms).
* **VAD**: energy-based with adaptive noise floor and ~300 ms hangover; sensitivity low/medium/
  high. Non-speech chunks are dropped before transcription (cheaper, fewer hallucinations).
* **Retention**: raw audio is **never** written to disk by default (`storeRawAudio: never`).
  `until_session_end`/`custom` keep chunks in memory/temp only as configured and purge them.

## Transcription
* **Apple (default)** — `SFSpeechRecognizer` per source with on-device recognition when the
  locale supports it (`supportsOnDeviceRecognition`); partial results stream; requests are
  rotated every ~55 s to respect the framework's one-minute limit. Works offline, no API key.
* **Cloud realtime** — WebSocket to an OpenAI/Azure realtime transcription endpoint
  (`session.update{type: transcription}`, `input_audio_buffer.append` with base64 PCM16,
  `conversation.item.input_audio_transcription.delta/completed`). Selected in Settings → Audio.
* **Mock** — fixture-driven for tests and developer mode.

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
