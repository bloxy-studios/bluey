# Native helper test harness (manual + scripted)

Exercises `bluey-helper` directly from a terminal on a **macOS 14+** machine —
no Tauri app needed. The helper speaks newline-delimited JSON on stdio
(see `docs/HELPER_PROTOCOL.md`), so `cat`/`printf` piped into the binary is a
complete test client.

> These tests exercise *real* ScreenCaptureKit / Vision / AX / CoreAudio /
> Speech paths. They cannot run in CI without a logged-in GUI session and TCC
> grants. Pure-logic unit tests live in
> `src-tauri/swift/BlueyHelper/Tests/` and run with `./scripts/test-helper.sh`.

## 1. Build & locate the binary

```bash
./scripts/build-helper.sh
HELPER=src-tauri/binaries/bluey-helper-aarch64-apple-darwin   # Apple Silicon
# HELPER=src-tauri/binaries/bluey-helper-x86_64-apple-darwin  # Intel
```

For a quick edit-compile-poke loop you can also run the debug build:

```bash
(cd src-tauri/swift/BlueyHelper && swift build)
HELPER=src-tauri/swift/BlueyHelper/.build/debug/bluey-helper
```

## 2. TCC caveat when running standalone

Run standalone, the helper is a child of your **terminal**, so macOS attributes
permission prompts/grants to Terminal/iTerm (the "responsible process") — grant
Screen Recording / Microphone / Accessibility to your terminal app for these
tests. Inside the packaged app the helper is a child of Bluey.app and prompts
are attributed to Bluey (usage strings live in the app's Info.plist).

## 3. Interactive smoke test

```bash
$HELPER
```

First line printed (stdout) must be the ready event:

```json
{"data":{"protocol":1,"version":"0.1.0"},"event":"helper.ready"}
```

(Key order is alphabetical — the helper encodes with sorted keys.)
Now type (or paste) one request per line, e.g. `{"id":"1","method":"helper.ping"}` —
expect `{"id":"1","result":{"pong":true,"uptimeMs":…}}` within 50 ms. Log lines
go to **stderr** only. `Ctrl-D` (EOF) makes the helper stop everything and exit 0.

## 4. Scripted runs with the .jsonl files

Each file in `requests/` is a valid stdin script. Pipe it in; keep stderr
separate so you can diff stdout:

```bash
$HELPER < tests/native/requests/smoke.jsonl 2>helper.log
```

| file | what it exercises | expected result lines |
|---|---|---|
| `smoke.jsonl` | ping/version/permissions/displays/audio devices | `pong:true`; `version` with `protocol:1`, `capabilities` incl. `capture`; a `Status` per permission; ≥ 1 display with `scaleFactor`; ≥ 1 input device (`isDefault:true` on one) |
| `permissions-request.jsonl` | TCC prompts (first run only) | one `{kind,status}` per request; `accessibility` opens the System Settings prompt |
| `capture.jsonl` | windows/frontmost + display/active-window/region capture | `Frame` objects: `c-4` has `path` under `~/Library/Caches/com.codewithabdul.bluey/frames/`, `c-5`/`c-7` have base64 `image`, all have 16-hex-char `hash`, `changed:true` first time; re-run without screen changes → `changed:false` |
| `ocr.jsonl` | Vision OCR on the fixture (see §6) | `blocks[]` with normalized top-left boxes; `text` contains the fixture's known text in reading order |
| `accessibility.jsonl` | AX snapshot (needs Accessibility granted) | `application` + `focusedElement` + `elements` (only protocol roles), `visibleText` ≤ 8000 chars, `truncated` flag |
| `observe-start.jsonl` / `observe-stop.jsonl` | screen.changed events | after start: switch windows/scroll → `{"event":"screen.changed","data":{hash,delta,…}}`; static screen → silence |
| `audio-start.jsonl` / `audio-stop.jsonl` | mic + system audio + VAD + levels + on-device STT | `audio.started`, periodic `audio.level`, `audio.chunk` (5/s at 200 ms; `isSpeech:true` while talking), `transcript.partial`/`transcript.final` while speaking; after stop: `audio.stopped {reason:"requested"}` |
| `shutdown.jsonl` | clean exit | `{"ok":true}` then process exits 0 |

Because piping a file sends all lines instantly, stateful flows need pauses.
Use a subshell to hold stdin open:

```bash
# observe for 15 s while you switch apps / scroll
( cat tests/native/requests/observe-start.jsonl; sleep 15; \
  cat tests/native/requests/observe-stop.jsonl; sleep 1 ) | $HELPER 2>helper.log

# record 12 s of mic+system audio with live transcription — speak into the mic
# and play a video, then check transcript.* events in the output
( cat tests/native/requests/audio-start.jsonl; sleep 12; \
  cat tests/native/requests/audio-stop.jsonl; sleep 2 ) | $HELPER 2>helper.log | tee audio-run.jsonl
```

Useful filters (`jq -c` keeps one object per line):

```bash
jq -c 'select(.event=="transcript.partial" or .event=="transcript.final")' audio-run.jsonl
jq -c 'select(.event=="audio.chunk") | {startMs:.data.startMs, isSpeech:.data.isSpeech, rms:.data.rms}' audio-run.jsonl
jq -c 'select(.id!=null)' audio-run.jsonl        # responses only
```

Checks worth doing on `audio.chunk`: `startMs`/`endMs` advance in exact
`chunkMs` steps with no gaps; `pcm16` is absent (set `"emitPcm":true` in
`audio-start.jsonl` to get base64 PCM, then `endMs-startMs` ms of 16-bit mono
at 16 kHz = 6400 bytes per 200 ms chunk after base64-decoding).

### capture.discard round-trip

```bash
# capture → verify the file exists → discard → verify it is gone
$HELPER <<'EOF' 2>/dev/null | jq -c .
{"id":"d-1","method":"capture.display","params":{}}
EOF
# take the "path" from d-1's Frame, then:
printf '{"id":"d-2","method":"capture.discard","params":{"path":"<PASTE PATH>"}}\n' | $HELPER 2>/dev/null
```

`capture.discard` with a path outside the frames dir must return an
`invalid_params` error — that is a test, not a bug.

### Error-shape checks

```bash
printf '{"id":"e-1","method":"nope.nope"}\n' | $HELPER 2>/dev/null
# → {"error":{"code":"method_not_found","kind":"not_supported",…},"id":"e-1"}
printf '{"id":"e-2","method":"capture.window","params":{}}\n' | $HELPER 2>/dev/null
# → error.kind == "invalid_params"
```

Revoke Screen Recording for your terminal in System Settings and re-run
`capture.jsonl`: every capture must fail with
`{"code":"permission_denied","kind":"permission","details":{"permission":"screenRecording"}}` —
never crash, never return a fake frame.

## 5. SIGTERM / EOF behaviour

```bash
$HELPER & PID=$!; sleep 1; kill -TERM $PID; wait $PID; echo "exit=$?"   # exit=0
printf '' | $HELPER; echo "exit=$?"                                      # EOF → exit=0
```

## 6. OCR fixture (`fixtures/ocr-sample.png`)

A binary PNG cannot live in this repo as text — generate it once on a Mac
(deterministic content, so assertions stay stable):

```bash
# Option A — render known text with built-in tools (no extra installs):
cat > /tmp/ocr-sample.txt <<'EOF'
Bluey OCR Fixture
The quick brown fox jumps over the lazy dog
0123456789 test@example.com
EOF
# TextEdit/Preview route: open the txt, screenshot the window…
# …or purely scripted with `screencapture` of a Terminal window showing it:
open -a TextEdit /tmp/ocr-sample.txt && sleep 2
screencapture -o -l "$(osascript -e 'tell app "TextEdit" to id of window 1')" \
  tests/native/fixtures/ocr-sample.png

# Option B — downscale any screenshot of crisp text with sips:
screencapture -i /tmp/raw.png                # interactively select a text region
sips --resampleWidth 1200 /tmp/raw.png --out tests/native/fixtures/ocr-sample.png
```

Expected once the fixture exists: `ocr.jsonl` request `o-1` returns every line
above in `text` (reading order top→bottom), each block with
`confidence ≥ 0.3` and `boundingBox.y` increasing per visual line.

## 7. What "pass" means overall

* stdout carries **only** JSON responses/events (validate: `$HELPER < requests/smoke.jsonl 2>/dev/null | jq -e . >/dev/null`).
* every request gets exactly one response with a matching `id`;
* `helper.ping` answers < 50 ms even while a capture or audio session runs;
* no raw audio ever appears on disk; temp frames only under the caches dir;
* helper exits 0 on EOF, SIGTERM and `helper.shutdown`.
