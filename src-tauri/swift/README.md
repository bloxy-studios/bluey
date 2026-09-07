# Bluey native helper (`bluey-helper`)

Swift sidecar owning every macOS-native capability: ScreenCaptureKit capture,
Vision OCR, Accessibility snapshots, CoreAudio/AVFoundation audio and Apple
Speech transcription. Spawned by the Rust backend as a Tauri sidecar; speaks
newline-delimited JSON on stdio per `docs/HELPER_PROTOCOL.md`. The WebView
never talks to it.

```
src-tauri/swift/BlueyHelper
├── Package.swift                  SwiftPM: lib BlueyHelperCore + exe bluey-helper + tests
├── bluey-helper.entitlements      hardened-runtime audio-input entitlement
├── Sources/BlueyHelperCore
│   ├── HelperApp.swift            wiring: router registration, lifecycle, SIGTERM/EOF
│   ├── Protocol/                  Envelope (Codable + JSONValue), JSONLinesIO, Router
│   ├── Permissions/               PermissionService (TCC status/request)
│   ├── Capture/                   DisplayService, WindowService, ScreenCaptureService,
│   │                              ScreenObserver, ChangeDetector (dHash), ImageEncoder,
│   │                              TempFrames, ShareableContent, CaptureTypes
│   ├── Vision/                    OCRService (+ pure OCRSorter reading order)
│   ├── Accessibility/             AXSnapshotService, FrontmostApp, AXValueHelpers, AXRoleFilter
│   ├── Audio/                     AudioSession (orchestrator), MicrophoneCapture,
│   │                              SystemAudioCapture, AudioDeviceService,
│   │                              VoiceActivityDetector, PCMChunker, LevelMeter
│   ├── Speech/                    SpeechTranscriber (SFSpeechRecognizer)
│   └── Util/                      Logger (stderr JSON), Clock (RFC3339 ms + monotonic)
├── Sources/bluey-helper/main.swift  emits helper.ready via HelperApp, then dispatchMain()
└── Tests/BlueyHelperCoreTests       pure-logic tests (no devices, no TCC)
```

Build: `./scripts/build-helper.sh` → `src-tauri/binaries/bluey-helper-{aarch64,x86_64}-apple-darwin`
(strip + codesign; `$APPLE_SIGNING_IDENTITY` or ad-hoc). Tests: `./scripts/test-helper.sh`.
End users never need Swift — binaries ship in the bundle.

## Threading model

* **stdin reader** (`JSONLinesIO`): `FileHandle.readabilityHandler` → serial
  parse queue → `Router`. Empty read = EOF → stop everything, exit 0.
* **stdout writer**: one serial queue; every response/event is one atomic
  line, written unbuffered. stderr (logs) has its own serial queue.
* **Router**: handlers run on a concurrent `userInitiated` queue. `helper.ping`,
  `helper.version`, `helper.shutdown` are answered inline on the parse queue so
  ping stays < 50 ms even when workers are saturated by a capture.
* Each stateful service serializes on its own queue: observer, audio session,
  mic engine, system-audio stream, each transcriber, level meter, CoreAudio
  listener. Cross-service handoff is message-passing (closures onto queues) —
  no shared mutable state without a queue/lock. Swift 5 language mode
  (`swiftLanguageVersions: [.v5]`) keeps strict-concurrency friction out.
* AppKit reads (NSScreen, NSWorkspace) hop to the **main queue**; the
  executable parks the main thread in `dispatchMain()`.
* AX traversal runs on a dedicated queue with a per-message timeout
  (`AXUIElementSetMessagingTimeout`, 100 ms) + a 250 ms total deadline, so an
  unresponsive app can never wedge the helper.
* Audio callbacks (AVAudioEngine tap / SCStream sample queue) do minimal work
  and hop onto the session queue for chunking/VAD/STT — the real-time thread
  never blocks on JSON encoding or stdout.

## Permissions / TCC attribution

The helper is a **child process of Bluey.app**, so TCC attributes prompts and
grants to the *responsible process* — the app bundle
(`com.codewithabdul.bluey`). Consequences:

* Usage strings live in the **app's** Info.plist (`NSMicrophoneUsageDescription`,
  `NSSpeechRecognitionUsageDescription`, `NSAccessibilityUsageDescription`);
  the helper binary carries none.
* Run standalone from a terminal (see `tests/native/README.md`), prompts are
  attributed to the terminal instead.
* Hardened runtime blocks mic input without the
  `com.apple.security.device.audio-input` entitlement — the build script signs
  the binary with `bluey-helper.entitlements`.
* Screen Recording has no tri-state API: `CGPreflightScreenCaptureAccess()`
  maps to granted/denied. A *fresh* Screen Recording grant only takes effect
  after the app restarts (macOS behaviour, Rust handles the relaunch UX).
* Accessibility is also boolean (`AXIsProcessTrusted`);
  `permissions.request{kind:"accessibility"}` triggers the system prompt via
  `AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt: true])`.

## macOS version requirements per feature

| feature | API | needs |
|---|---|---|
| screenshots | `SCScreenshotManager.captureImage(contentFilter:configuration:)` | **macOS 14.0** (hence deployment target) |
| window/display enumeration | `SCShareableContent.excludingDesktopWindows(_:onScreenWindowsOnly:)` | 12.3 |
| screen-change observer | `SCStream` + `minimumFrameInterval` | 12.3 |
| system audio | `SCStreamConfiguration.capturesAudio` / `excludesCurrentProcessAudio` | 13.0 |
| OCR | `VNRecognizeTextRequest` | 10.15 (revision 3 languages: 13+) |
| AX snapshot | `AXUIElement*` | any |
| mic capture | `AVAudioEngine` input tap + `AVAudioConverter` | any |
| on-device STT | `SFSpeechRecognizer.supportsOnDeviceRecognition` | 10.15; model availability varies per locale |
| default-input listener | `AudioObjectAddPropertyListenerBlock` + `kAudioObjectPropertyElementMain` | 12.0 |

Everything compiles against a **14.0 deployment target** with no availability
gates. Optional future work: `SpeechAnalyzer` (macOS 26) removes the SFSpeech
1-minute cap — adopt behind `#available(macOS 26, *)`, keeping SFSpeech as the
default path (noted in `SpeechTranscriber.swift`).

## Known limitations

* **SFSpeech ~1-minute request cap** — requests rotate every ~55 s and after
  each final; a word straddling the rotation boundary can be clipped.
  `startMs/endMs` stay correct across rotations (sample-count epochs).
* **On-device speech models** are per-locale downloads; if unavailable the
  transcriber falls back to server-based recognition unless
  `transcription.onDevice` demanded it (then recognition may be unavailable →
  `audio.error{code:"speech_unavailable"}`). Partial results carry no
  confidence (`confidence` null; finals carry the segment average).
* **No speaker diarization** — speaker labels derive from the audio *source*
  in Rust (`microphone` → "You", `system` → "Speaker"). Two people on the
  system-audio side are indistinguishable here.
* **SCStream audio sample rate**: officially 8/16/24/48 kHz. We request 16 kHz
  mono directly; a defensive downmix/linear-resample path covers deviating
  delivery formats. ScreenCaptureKit refuses audio-only streams, so a discarded
  2×2 @ 1 fps video output rides along.
* **audio.pause** freezes the chunk clock (`startMs/endMs` are sample-count
  based): paused audio is dropped, not buffered, and the timeline resumes where
  it stopped — by design, Rust treats them as a contiguous stream.
* **Window titles** via CGWindowList (`app.frontmost`, windowId matching) need
  Screen Recording; without it `title` is null. AX has no public windowId, so
  the snapshot's `windowId` is a best-effort CGWindowList match (title, then
  closest bounds).
* **`capture.window` of minimized / other-Space windows** may return stale or
  empty content (`SCContentFilter(desktopIndependentWindow:)` behaviour).
* **Screenshot change detection** uses a fixed 0.04 dHash threshold per target;
  `observe.start` takes an explicit `minDelta`.
* **Privacy invariants**: raw audio only ever exists in in-flight chunk
  buffers (never written to disk); frames only under
  `~/Library/Caches/com.codewithabdul.bluey/frames/` (stale > 1 h purged at
  startup, `capture.discard` validates paths against that directory); stderr
  logs never contain audio, transcripts, OCR text or image bytes.
