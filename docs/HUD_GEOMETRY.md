# HUD surface and native-frame geometry

The visible Cluely-inspired HUD styling is unchanged: 690-point default surface width,
16-point radius, the existing colors/blur, `b` mark and `0 8px 32px` CSS shadow.

## Size contract (no new command or state fields)

- **Appearance width** is the visible surface's border-box width.
- **PanelState width/height**, `panel_resize`, and the optional height in
  `panel_set_expanded` describe the transparent **native frame** in logical points.
- The measured frame includes **24 top / 32 right / 40 bottom / 32 left** points of
  shadow room. Keep `features/hud/geometry.ts` and `bluey-protocols/src/panel.rs` in sync.
- The idle surface is currently 111 points: 56 input + 52 toolbar + 1 divider + 2 outer
  borders. The startup frame is therefore **754 × 175**. This is a fallback, not a fixed
  idle height. Listening can add transcript rows even when `expanded` is false.
- Height reports already include the insets and borders. Rust must **not add them again**.
  Initial and observer measurements both use the outer frame's border-box, rounded up
  once; `contentRect` is not a substitute. The appearance animation is on the inner
  surface, not the measured frame.
- Native width comes from the actual WebView viewport, so work-area clamping cannot
  leave a wider CSS surface clipped at the right edge. Browser preview still uses the
  appearance width with `max-width: 100%`.

## Native minimums and safe manual sizing policy

- The launch config and `PanelState` defaults use **754 × 175**. Static minimums are
  **484 × 175**: 420 visible width + the horizontal insets, and the idle frame height.
- Before attaching or fitting a window on a display, Rust calls `set_min_size` with
  `(min(484, work.width), min(175, work.height))`, in logical points. It updates the
  constraint **before `set_size`**, so a 480-point-wide display really can fit without
  permitting a 1-point frame on an ordinary display. Constraints are refreshed on scale
  changes and restored when moving back to a larger work area.
- **Height is content-owned; width is Appearance-owned. Native edge resizing is disabled.**
  This intentionally makes the config and runtime agree with the existing non-resizable
  NSPanel style mask. It avoids adding a manual-height mode that can hide the footer or
  lock a collapsed panel out of growing. Drag-to-move and Appearance width controls remain
  available. `panel_resize` is a low-level frame command, not a persistent manual-size mode;
  the UI does not use it. A display fit does not permanently change the width preference.
- Native move/resize/scale/focus events now actually call `sync_from_window`. After a
  120ms settling interval it reads the **live** rect, not a stale event payload. Panel
  mutations (including persistence) are serialized, preventing a deferred drag/resize or
  opacity snapshot from overwriting a newer size. Identical fitted state is a no-op.
  Native operation comparisons use backing-pixel equivalence to avoid fractional-scale
  resize loops; the work-area sizes and fitting calculations remain exact logical values.

## Growth, caps, and scroll ownership

The outer frame has natural content height, **not `height: 100%`**. The surface has a
620-point maximum, reduced to the current monitor's work-area height minus the vertical
insets. It is never capped to the current auto-sized native `innerHeight`: doing that
would make a collapsed window unable to grow again.

The composer and toolbar are non-shrinking siblings. The response flex wrapper and its
scroll viewport have `min-height: 0`. The 120-point natural reading minimum is on the
inner content, not the viewport; there is no second fixed 460-point cap. Extra response
text scrolls in that viewport, including with the existing scroll shortcuts. The transcript
strip remains bounded to its existing one/four truncated lines; it does not steal scroll
ownership from the response. Extremely short areas that cannot fit the non-scrolling
chrome plus transcript are not a useful interactive size; verify the practical small-display
cases below rather than inferring native layout from jsdom.

Height notifications are coalesced every 80ms (not an indefinitely postponed trailing
stream debounce), duplicates are suppressed, and cleanup drops scheduled/obsolete
observer callbacks. Zero/invalid observations cancel a pending size; a transient native
failure retries a static measurement once, not indefinitely. A work-area change rechecks
the frame even if its height is unchanged. The store sends one native resize at a time,
retaining only the newest queued measurement, and recovers from synchronous transport
failures too. Command/load replies cannot overwrite a newer `panel.state` event, and a
slow/overlapping load cannot undo a resize reply even when no event arrives. The backend
skips native size/position calls when unchanged and avoids repeated persistence/events
for identical fitted state.

## Native boundaries

- Fitting reduces the **size as well as the origin** to the work area; a large frame must
  not merely be pinned to its top-left and left overflowing.
- A content resize selects the display from the **current** frame, not a hypothetical
  very tall response whose centre could fall on a different display. The top-left origin
  is preserved unless fitting requires a move. Width-setting changes keep the centre.
- Geometry helpers and their tests live in `bluey-protocols` and run on Linux. No AppKit
  operations, new dependencies, permissions, commands or state fields are required there.
- `lib/tauri/work-area.ts` owns direct monitor/window API access and native subscriptions;
  feature code consumes logical dimensions and disposal callbacks only. Tauri
  `Monitor.workArea` dimensions are physical; the bridge converts using that monitor's
  `scaleFactor` exactly once. Invalid sizes/scales fall back to independent screen bounds.
  CSS reports are already logical points and must never be multiplied by `devicePixelRatio`.
- If native monitor enumeration temporarily fails, Rust preserves the origin and does not
  use the current auto-sized frame as a new growth cap. Screen-edge fitting is best-effort
  until a monitor is available again; it cannot be verified from missing display data.
- The existing native origin conversion is retained: Tao 0.35.3 reads `NSWindow.frame`
  in logical coordinates before converting by the window scale, and Tauri runtime-wry
  2.11.4 converts `NSScreen.visibleFrame` using each monitor's own scale. This change does
  not introduce an alternate global mixed-DPI coordinate system.
- CSS applies the opacity preference **once**; NSPanel alpha stays 1.0. Native pin level,
  Spaces/fullscreen collection behavior, drag initiation and content protection are untouched.

## Focused automated checks

```sh
bun run test tests/ui/hud-geometry.test.tsx tests/ui/hud-work-area.test.ts \
  tests/ui/panel-store.test.ts tests/ui/hud-layout.test.tsx tests/ui/hud.test.tsx \
  tests/integration/hud-frame-contract.test.ts
cd src-tauri && cargo test -p bluey-protocols panel::tests
```

The UI tests verify measurement, sequencing, cleanup, the Tauri import boundary and
scroll/DOM boundaries. Cross-language contract tests compare CSS insets, config, Rust
and mock frame defaults; pure Rust tests exercise native minimums/fitting, including
fractional sizes and fractional-scale no-ops. The browser mock models frame measurement
semantics, not OS constraints or monitor movement. These tests do not validate real
browser layout or NSPanel/AppKit behavior; those need the checks below.

## macOS manual QA (required before release)

1. On Retina and a 1× external monitor, compare visible width at 520 / 690 / 960 and
   opacity at 40 / 70 / 100%. The preference should not be squared, text/logo should be
   intact, and all four shadow edges should have breathing room.
2. Idle → listening (waiting, partial, one to four final lines) → collapse transcript →
   stop listening. Frame height must grow and shrink with the strip while chat stays idle.
3. Short answer → continuously streaming prose, long code/table/diagram → follow-up →
   stop → new chat. After every shrink it must grow again; input and toolbar stay visible,
   with only the response viewport scrolling. Check keyboard scroll and scroll-to-bottom.
4. Repeat with enlarged response text, blur off, light/dark, reduced motion and a short
   work area (for example **600 × 360 logical points**, and a 600-point-tall display with
   a large Dock). Exercise 960 width on a narrower work area and the toolbar menus.
5. At each display edge, grow/shrink and use ⌘ arrows. No unnecessary drift; at the bottom
   the frame moves up only enough to fit. Drag between 1×/2× displays to the left, right,
   above and below; change scaling, move the Dock, disconnect a display, then hide/show.
6. Verify edge dragging does **not** resize the panel (including a vertical drag during
   a long response); Appearance width still works and new content grows normally. Move
   from a normal work area to one under 484 points wide, then back: the runtime minimum
   and preferred width must recover. Test an actual fractional scale if available for
   resize/event feedback loops. Check remembered position, pin/always-on-top, fullscreen
   Spaces, and content protection. Browser preview cannot establish these native behaviors.
