# Bluey Native Helper Protocol (Swift ⇄ Rust)

The native macOS helper (`bluey-helper`, Swift, `src-tauri/swift/BlueyHelper`) is a
Tauri **sidecar** spawned by the Rust backend. All native capabilities that depend on
ScreenCaptureKit, Vision, Accessibility, AVFoundation and Speech live here. The Rust
side (`src-tauri/src/sidecar/`) is the only client; the WebView never talks to it.

## Transport

* **stdio, newline-delimited JSON (JSON Lines)**. One JSON object per line, UTF-8.
* Rust → helper: **requests** on the helper's `stdin`.
* Helper → Rust: **responses** and **events** on `stdout`. `stderr` is reserved for
  structured log lines (`{"level":"info","message":"..."}`) — never for protocol data.
* Requests are asynchronous and may complete out of order; correlate by `id`.
* Large binary payloads (images, PCM audio) are **base64** in JSON or written to a
  temp file whose path is returned (`path`). Images default to temp files; audio
  chunks are inline base64 (≤ ~64 KB per chunk).
* The helper must never block its stdin reader: long-running work runs on
  background queues; `helper.ping` must always answer within 50 ms.
* On `EOF` of stdin the helper stops all capture/audio and exits 0.

### Envelope

```jsonc
// request
{ "id": "r-42", "method": "capture.display", "params": { ... } }
// success response
{ "id": "r-42", "result": { ... } }
// error response
{ "id": "r-42", "error": { "code": "permission_denied", "message": "Screen Recording not granted", "kind": "permission", "details": { "permission": "screenRecording" } } }
// event (no id)
{ "event": "audio.chunk", "data": { ... } }
```

`error.kind` ∈ `permission | capture | audio | transcription | not_supported | internal | invalid_params`.
`error.code` is a stable snake_case identifier. Rust maps these to `BlueyError`.

## Methods

### helper
| method | params | result |
|---|---|---|
| `helper.ping` | – | `{ "pong": true, "uptimeMs": n }` |
| `helper.version` | – | `{ "version": "x.y.z", "macos": "15.1", "arch": "arm64", "capabilities": ["capture","ocr","accessibility","audio.microphone","audio.system","speech.onDevice"] }` |
| `helper.shutdown` | – | `{ "ok": true }` then exit |

### permissions
| method | params | result |
|---|---|---|
| `permissions.status` | – | `{ "screenRecording": Status, "microphone": Status, "accessibility": Status, "speechRecognition": Status }` |
| `permissions.request` | `{ "kind": "microphone" \| "screenRecording" \| "accessibility" \| "speechRecognition" }` | `{ "kind": ..., "status": Status }` (may show the system prompt; `accessibility` opens the prompt via `AXIsProcessTrustedWithOptions`) |

`Status` ∈ `granted | denied | not_determined | restricted | unknown`. Note: notifications
permission is handled in Rust (tauri-plugin-notification), not by the helper.

### displays / windows
| method | params | result |
|---|---|---|
| `displays.list` | – | `{ "displays": [ { "id": "69733382", "name": "Built-in Retina Display", "width": 1728, "height": 1117, "x": 0, "y": 0, "scaleFactor": 2, "isMain": true } ] }` — sizes are in **points**; `x/y` in the global (Quartz) coordinate space |
| `windows.list` | `{ "onScreenOnly": true }` | `{ "windows": [ { "windowId": 1234, "title": "…", "ownerName": "Google Chrome", "bundleId": "com.google.Chrome", "pid": 512, "bounds": {x,y,width,height}, "onScreen": true } ] }` (excludes Bluey's own windows) |
| `app.frontmost` | – | `{ "application": { "name", "bundleId", "pid" }, "window": { "title", "windowId", "bounds" } }` |

### capture (ScreenCaptureKit)
Common params: `format` (`"jpeg"` default \| `"png"`), `quality` (0–1, default 0.8),
`maxDimension` (default 1600, longest side in pixels after downscale), `inline`
(default false → write temp file), `changeDetection` (default true), `excludeSelf` (default true —
Bluey's own windows are excluded from the content filter).

| method | params | result |
|---|---|---|
| `capture.display` | `{ "displayId"?: string, ...common }` | `Frame` |
| `capture.window` | `{ "windowId": number, ...common }` | `Frame` |
| `capture.region` | `{ "displayId"?: string, "rect": {x,y,width,height} (points, display-local), ...common }` | `Frame` |
| `capture.activeWindow` | `{ ...common }` | `Frame` (frontmost app's focused window) |
| `capture.discard` | `{ "path": string }` | `{ "ok": true }` |
| `observe.start` | `{ "intervalMs": 1500, "displayId"?: string, "minDelta": 0.04 }` | `{ "ok": true }` — starts a low-FPS `SCStream`, computes a perceptual hash per sampled frame, emits `screen.changed` only when the Hamming distance ratio ≥ `minDelta` |
| `observe.stop` | – | `{ "ok": true }` |

```jsonc
// Frame
{
  "id": "f-…", "path": "/…/bluey/frames/f-….jpg", "image": null /* base64 when inline */,
  "mimeType": "image/jpeg", "width": 1600, "height": 1035, "displayId": "69733382",
  "scaleFactor": 2, "capturedAt": "2026-09-07T12:00:00.000Z",
  "hash": "a3f0…", "changed": true, "durationMs": 87
}
```
`changed:false` means the dHash matched the previous frame of the same target (within
`minDelta`); the frame is still returned (callers decide whether to skip OCR/AI).

Events: `screen.changed` → `{ "hash": string, "delta": number, "displayId"?: string, "at": iso }`.

### ocr (Vision)
| method | params | result |
|---|---|---|
| `ocr.recognize` | `{ "path"?: string, "image"?: base64, "level": "fast" \| "accurate", "languages": ["en-US"], "minConfidence": 0.3 }` | `{ "blocks": [ { "text", "confidence", "boundingBox": {x,y,width,height} /* normalized 0–1, top-left origin */ } ], "text": "joined in reading order", "width", "height", "durationMs" }` |

### accessibility (AX APIs)
| method | params | result |
|---|---|---|
| `accessibility.snapshot` | `{ "maxDepth": 6, "maxElements": 150, "includeSelectedText": true }` | `AccessibilitySnapshot` |

```jsonc
{
  "application": { "name": "Code", "bundleId": "com.microsoft.VSCode", "pid": 913 },
  "window": { "title": "main.rs — bluey", "windowId": 88, "bounds": {…} },
  "focusedElement": { "role": "AXTextArea", "label": "Editor", "value": "…(truncated to 4000 chars)", "position": {x,y}, "size": {w,h}, "actions": ["AXPress"], "focused": true, "depth": 0 },
  "elements": [ /* only text-bearing / interactive elements: AXStaticText, AXTextField, AXTextArea, AXButton, AXLink, AXMenuItem, AXCheckBox, AXRadioButton, AXPopUpButton, AXHeading, AXCell, AXWebArea(url) */ ],
  "selectedText": "…",
  "visibleText": "deduplicated visible text, ≤ 8000 chars",
  "truncated": false,
  "capturedAt": iso
}
```
The helper must bound the traversal (`maxDepth`, `maxElements`, 250 ms time budget) and
never dump the whole tree.

### audio (AVAudioEngine + ScreenCaptureKit audio)
| method | params | result |
|---|---|---|
| `audio.devices` | – | `{ "devices": [ { "id", "name", "isDefault", "kind": "input" } ] }` |
| `audio.start` | `AudioStartParams` | `{ "ok": true, "microphone": bool, "systemAudio": bool, "sampleRate": 16000 }` |
| `audio.stop` | – | `{ "ok": true }` |
| `audio.pause` / `audio.resume` | – | `{ "ok": true }` |
| `audio.testMicrophone` | `{ "deviceId"?: string, "durationMs": 1500 }` | `{ "peakLevel": 0.42, "ok": true }` |

```jsonc
// AudioStartParams
{
  "microphone": { "enabled": true, "deviceId": "AppleHDAEngineInput:…" },
  "systemAudio": { "enabled": true },            // SCStream with capturesAudio, excluding Bluey's own process
  "sampleRate": 16000,                           // helper resamples to mono PCM16 at this rate
  "vad": { "enabled": true, "sensitivity": "medium" },  // energy VAD w/ hangover; sensitivities map to RMS thresholds
  "emitPcm": false,                              // true → emit `audio.chunk` with pcm16 base64 (cloud STT path)
  "chunkMs": 200,
  "transcription": {                             // on-device STT inside the helper (Apple Speech)
    "enabled": true, "locale": "en-US", "onDevice": true, "sources": ["microphone", "system"]
  },
  "levels": { "enabled": true, "intervalMs": 100 }
}
```
Events:
* `audio.started` → `{ "microphone": bool, "systemAudio": bool, "device": Device? }`
* `audio.stopped` → `{ "reason": "requested" | "device_lost" | "error" }`
* `audio.level` → `{ "microphone": 0..1, "system": 0..1 }`
* `audio.chunk` → `{ "source": "microphone" | "system", "pcm16": base64?, "sampleRate": 16000, "startMs": n, "endMs": n, "isSpeech": bool, "rms": 0..1 }` (`pcm16` only when `emitPcm`)
* `audio.deviceChanged` → `{ "devices": [...], "currentInput": Device? }` (helper re-routes the engine automatically when the default input changes)
* `audio.error` → `{ "code", "message", "kind": "audio" }`
* `transcript.partial` / `transcript.final` → `{ "source": "microphone" | "system", "text", "startMs", "endMs", "confidence": 0..1?, "locale": "en-US" }`
  * `startMs/endMs` are milliseconds since `audio.start`.
  * Speaker labelling is done in Rust from `source` (`microphone` → "You", `system` → "Speaker"). The helper never guesses speakers.

### notifications
Handled in Rust via `tauri-plugin-notification`; not part of this protocol.

## Lifecycle & robustness
* Rust spawns the helper at app start (after auth) and restarts it with backoff on crash;
  emits `helper.status` to the frontend.
* The helper writes `{"event":"helper.ready","data":{"version":…}}` as its first line.
* Requests time out on the Rust side (capture 3 s, ocr 5 s, ax 1 s, audio.start 5 s).
* The helper must handle `SIGTERM` by stopping streams and flushing stdout.
* Temp frames live in `~/Library/Caches/com.codewithabdul.bluey/frames/` and are deleted by
  Rust after use (`capture.discard`) or by the helper on startup (stale > 1 h).

## Versioning
`helper.version.protocol` = `1`. Rust refuses to start with an incompatible major version.
