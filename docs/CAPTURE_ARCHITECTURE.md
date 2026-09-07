# Screen Capture, OCR & Accessibility

```
 shortcut / HUD ──► Rust CaptureManager ──► helper capture.* ──► frame (temp JPEG, dHash)
                          │                                            │
                          ├──► helper ocr.recognize (Vision) ◄─────────┘
                          ├──► helper accessibility.snapshot (AX)
                          └──► ContextSnapshot (trimmed, adapter hints) ──► engine
```

## Capture (ScreenCaptureKit, macOS 14+)
* **On demand**: `SCScreenshotManager.captureImage(contentFilter:configuration:)` for the
  active display (default), a window, a region (`sourceRect`) or the active window. Bluey's own
  windows are excluded from the content filter so the HUD never appears in captures.
* **Multi-monitor / Retina**: displays enumerated with points, origin and scale factor; the
  frame is rendered at native pixels and downscaled so the longest side ≤ `maxImageDimension`
  (default 1600 px), JPEG q0.8. Frames are written to
  `~/Library/Caches/com.codewithabdul.bluey/frames/` and discarded after use.
* **Change detection**: a 64-bit difference hash per target; `changed=false` when the Hamming
  ratio is below `minDelta`. The engine skips vision/OCR work for unchanged screens and reuses
  the cached OCR.
* **Smart observation (off by default)**: a low-FPS `SCStream` (default 1.5 s interval)
  computing dHash per sampled frame; emits `screen.changed` only on material change. No frame
  is sent to a model automatically — observation only refreshes the cached context so ⌘↵ is
  faster and proactive preparation has fresh OCR.
* **Cancellation**: a newer capture request supersedes an in-flight one.

## OCR (Vision)
`VNRecognizeTextRequest` — `fast` (default, ~100–300 ms for a 1600 px frame) or `accurate`
(with language correction). Output: blocks with confidence and normalized top-left-origin
bounding boxes, plus text joined in reading order. Cached per frame hash.

## Accessibility (AX APIs)
`accessibility.snapshot` collects the frontmost application, focused window, focused element
(role/label/value/position/size/actions), selected text, and a bounded set of text-bearing and
interactive elements (depth ≤ 6, ≤ 150 elements, 250 ms deadline). The result complements the
screenshot: OCR gives pixel-level text, AX gives semantic structure (which field is focused,
what's selected, what the buttons say). The tree is never dumped whole into a prompt.

## Application adapters
`bluey_core::context::detect_adapter` maps bundle ids / window titles to `generic`, `browser`,
`vscode`, `terminal`, `zoom`, `google-meet`, `teams`, `slack` and adds hints
(`meetingDetected`, `codingContext`, `browserTitle`). Adapters enrich context; nothing depends
on them.

## Snapshot hygiene
`trim_snapshot` bounds OCR (12k chars), AX visible text (8k), elements (150), transcript window
(300 s / 200 segments) and removes OCR lines duplicated by AX text before the TS layer scores
and budgets the context.

## Privacy controls
Capture target (display / active window / region), observation mode (manual only by default),
screenshot retention (off by default), Privacy display mode (content protection — ADR 0006),
and a visible `◌ Reading screen` state whenever a capture happens.
