# Native HUD menus and keyboard interactions

## Status: Rust-owned AppKit popup implemented; macOS runtime verification pending

In the Tauri runtime, the Mode and Session buttons now open a real `NSMenu`
through one display-only Rust command, `hud_menu_popup`. AppKit draws/tracks this
menu outside the compact NSPanel's webview; no panel resize or web portal is used
to work around clipping. Browser previews retain the existing Radix menus and
MockTransport. Runtime detection is independent of simulated application data.

The native path compiles against the existing locked dependencies. This is
**compile/contract/lifetime-ownership evidence**, not a tested macOS NSPanel
session, measured memory usage, Retina placement, or VoiceOver verification.
The native checklist below remains required on macOS.

Scope: HUD menu presentation/interaction only. Labels, order, active checks,
disabled items, header/separator grouping, Manage/History destinations and domain
actions are unchanged. Fixed SF Symbol equivalents preserve the existing icon
cues; End session retains a semantic red title. AppKit supplies native menu
material/typography/scrolling/tracking; the web design is unchanged. No panel
geometry, Settings, capture/privacy configuration, AI, auth, audio, storage,
providers, dependency versions, workflows or capabilities change. Only the
existing `objc2-app-kit` dependency gains explicit framework features.

## Installed source and selection-order audit

Inspected locally: JS `@tauri-apps/api` **2.11.1**; locked Rust `tauri` **2.11.5**,
`muda` **0.19.3**, `objc2` **0.6.4**, AppKit/Foundation **0.3.2**.

### The JS menu plugin leak is bypassed, not hidden

`@tauri-apps/api/menu/base.js:22–73` creates a `Channel` per new menu/item.
`tauri-2.11.5/src/menu/plugin.rs:361–466,889–930` inserts those channels into
`MenuChannels` without an unregister operation. Resource close removes a
*different* resource-table entry (`src/resources/plugin.rs:13–23`). Upstream
confirms this in [1417768 / #15679](https://github.com/tauri-apps/tauri/commit/1417768f9941f6ee998e7b5c9fe609ada61a6727).

The new path never imports the JS menu API, creates an IPC `Channel`, allocates a
Tauri menu/resource-table entry or registers per-popup event handlers. It needs
no upgrade, arbitrary timeout, resource cache or delayed cleanup workaround.

### Why not public `tauri::menu` plus a handler?

This was inspected separately from the JS issue:

- `tauri-2.11.5/src/menu/menu.rs:43–80` calls muda through
  `run_item_main_thread!`; `src/menu/mod.rs:25–38` waits for that task. This
  provides a popup close boundary, but does not return the selected ID.
- `muda-0.19.3/src/platform_impl/macos/mod.rs:1025–1131` receives AppKit
  target-action and sends `MenuEvent`; `1190–1215` invokes the native popup.
- **However**, `tauri-2.11.5/src/app.rs:2346–2352` installs a global handler that
  posts selection through `proxy.send_event(EventLoopMessage::MenuEvent(...))`.
  A Tauri `on_menu_event` callback is not the synchronous AppKit action callback;
  its delivery cannot be assumed to precede popup task completion.
- `muda-0.19.3/src/lib.rs:491–529` stores the handler in a `OnceCell`; after Tauri
  installs it the alternative receiver gets no events. No global handler
  replacement, extra event-loop pump, per-popup `on_menu_event` or timing guess
  is used. The existing tray's event handling is untouched.

### Direct public AppKit target-action

`src-tauri/src/platform/hud_menu_macos.rs` uses public AppKit APIs via objc2:

- `objc2-app-kit-0.3.2/src/generated/NSMenu.rs:77–82,165–173`: `NSMenu` is
  `MainThreadOnly`; popup synchronously returns a boolean.
- [Apple's popup contract](https://developer.apple.com/documentation/appkit/nsmenu/popup(positioning:at:in:)):
  true when tracking ended with selection, false when cancelled. Coordinates
  are in the supplied view's own coordinate system.
- `NSMenuItem.rs:82–92,350–380`: selector initialization, weak target, action and
  integer tag. Our fixed `chooseHudMenuItem:` selector writes a popup-local
  `Cell<Option<usize>>` synchronously, not a posted Tauri event. After popup
  returns, Rust reads it only if tracking returned true, rechecking that the tag
  identifies an enabled item. Highlighted items are never treated as selections.
- `NSMenuItem.rs:65–74,266–275`: separators, noninteractive headers and checks.
  Headers fit the app's existing **macOS 14 minimum**. Explicit framework
  features cover menu/item/view/check-state and existing visual cues; versions
  and both lockfiles are unchanged.
- `tauri-2.11.5/src/window/mod.rs:1649–1663` returns the owning window's content
  NSView; `WebviewWindow::ns_view` delegates to it. We retain that view and its
  NSWindow on the main thread before tracking. No raw handle crosses threads.

`BlueyHudMenuTarget` is one ordinary Objective-C class registered once, not
swizzling or a class per open. Each popup owns its own target instance. Item
references to it are weak, so there is no menu/target retain cycle.

## Bounded command and explicit ownership

`commands::hud_menu::hud_menu_popup(window: WebviewWindow, request)` is async.
Tauri injects the invoking window; JS cannot choose an owner. Validation accepts
only `main`; other windows fail despite central application-command registration.
The result is `string | null` after tracking/cleanup (`null` also covers busy or
hidden HUD), or a typed validation/dispatch/view error. No plugin menu
permission is needed. The optional read-only window focus query is already
covered by the existing `core:default`; no ACL additions are made.

`hud-menu-types.ts` mirrors `bluey-protocols::hud_menu`; both suites use the same
round-trip JSON fixture:

- 1–128 display items, only item/label/separator, no nested menus.
- Unique nonempty IDs up to 64 ASCII alphanumeric/underscore/hyphen bytes.
  Frontend-generated `hud-N` IDs are popup-local; domain IDs stay in JS.
- Single-line labels up to 80 Unicode scalar values, with no control characters.
  Display normalization never modifies stored mode/session names.
- Finite client logical `x,y` in `[0,16384]`, also checked against the actual
  native content view. End alignment uses the button's right edge minus menu
  width. Respect NSView flipping; no screen coordinates or DPR multiplication.
- Enabled/check/destructive flags and five fixed icon enum values. Unknown
  fields/types/icon names fail deserialization. No actions, URLs, selectors,
  callbacks, shell strings or filesystem images cross this boundary.

Lifetime sequence:

1. Validate before AppKit allocation. Acquire one non-queuing Rust `PopupGate`
   shared by both menus before main-thread dispatch.
2. Move its permit and an owning WebviewWindow clone into the main-thread task.
   Dropping the IPC receiver cannot unlock a menu still tracking. Failed
   dispatch drops the unqueued closure/permit.
3. Create/use/drop AppKit objects within `autoreleasepool`, after checking
   `MainThreadMarker`. A hidden HUD is not raised to service stale input. A visible
   non-key HUD is accepted: non-activating panels deliberately need not become key
   when a toolbar button is clicked, so a key-window precondition would swallow
   the first click. No native `setFocus`, make-key or app activation call is added.
4. `Popup` retains every created item and its target through tracking.
   `Popup::drop` clears actions/targets, removes items and releases ownership,
   including partial construction/unwind. Normal validation/view/dispatch
   failures use the same RAII paths; allocation aborts are not recoverable.
5. After tracking returns, resolve only an enabled ID from the validated
   snapshot. Drop native objects/drain the pool, release the lease, then send the
   Rust oneshot result. This oneshot is not a Tauri IPC `Channel`. There are no
   application menu/resource/handler registrations to orphan per open.

## Frontend interaction and action parity

`src/lib/tauri/native-hud-menu.ts` owns serialization, IPC and focus restoration;
`bluey.hudMenu.popup` uses the normal typed commands/api/transport path. The
command is covered by command-surface parity. The browser mock's direct command
returns cancellation; browser HudMenu never calls it and retains Radix behavior.

- The shared frontend guard is acquired synchronously before serialization/IPC.
  Repeated Mode/Session activations are suppressed, not queued with stale anchors.
  Success, cancellation, invalid IDs and creation/focus-query failure release it.
  It stays held through the close-time focus query.
- Radix `Slot` composes refs/handlers onto the actual native-mode IconButton,
  with `aria-haspopup`, `aria-expanded`, no additional button/span. Pointer,
  Enter, Space and ArrowDown open it; repeats, composition/229 and prevented
  events cannot open another popup. Native open state participates in the
  existing HUD overlay keyboard scope. AppKit owns Escape/outside dismissal.
- Only an ID in the open-time enabled-action map can invoke `onSelect`.
  Mode/Manage use the same calls; Session still delegates to `session-actions.ts`
  with its existing audio/session coordination and guarded error toasts.
- Restore only DOM trigger focus it **already had** at open: connected/enabled
  trigger, focused document and a successful read-only native `isFocused()`
  query, then recheck DOM state (including whether another control took focus).
  Restore before the selected action can open Settings. Never call native `setFocus()`.
- Unmount does not dispose live native resources or release the guard early.
  Late completion cannot invoke actions/state updates on a departed trigger.
  A native failure surfaces an error rather than silently falling back to a
  clipped web menu in the desktop runtime.

Existing IME/held-key/input/new-chat fixes and browser theme/scrolling/tooltip
styling remain. No domain implementation or shortcut binding changes.

## Automated evidence — Linux host, not macOS runtime

Validated on this branch:

| Check | Result |
| --- | --- |
| `bun run typecheck`, `bun run lint`, `bun run build` | Passed |
| `bun run test` | **517 passed**, 62 files (35 new native adapter/trigger tests) |
| Chromium/Radix fallback script | **17 passed**, 0 failed |
| Host Rust tests | **286 passed**: core 85, protocols 130 (13 new HUD tests), storage 71 |
| Rust fmt + host clippy `--all-targets -- -D warnings` | Passed |
| Darwin app check + Darwin clippy `-- -D warnings` | Passed, compile-only |

```sh
bun run typecheck
bun run lint
bun run test
bun run build
CARGO_TARGET_DIR=/path/to/host-target bash scripts/check-rust.sh --darwin
```

Additional Darwin clippy uses the same check-only environment as that script:
`cargo clippy --target aarch64-apple-darwin --no-default-features --features dev-tools -- -D warnings`.
The script's fake Darwin C compiler and cached sidecar placeholders on Linux
are for **type checking only**, never distributable builds or runtime evidence.

New regressions cover payload round-trip/bounds, main-window restriction,
selection allowlisting, permit release and dropped receiver lifetimes; real typed
frontend serialization with mocked native IPC/focus; deferred/rejected requests,
cross-menu serialization, unmount, button/ref/key semantics, and unchanged domain
actions. These mocks do not run AppKit or observe its allocator.

Actual Chromium/Radix fallback regression script (no screenshots/installs):

```sh
PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs bun tests/browser/hud-interactions.mjs
```

It starts/stops Vite and exercises pointer/keyboard/actions/focus/overflow/tooltips
using MockTransport. `document.hasFocus()` is stubbed for its focus-loss check,
not an external macOS app. Floating Radix content is exercised in Chromium, not
jsdom, where opening it stalls in this environment. Native unit tests do not
pretend jsdom is an AppKit tracking loop.

## Native verification checklist — not executed here

1. On macOS 14+, use the actual compact NSPanel (idle and expanded). Confirm
   both menus extend beyond it with no panel size change. Test pointer,
   Enter/Space/ArrowDown, Escape/outside click and disabled/checked items.
2. Check long names/lists, native scrolling, light/dark styling, icon/red-title
   cues, keyboard focus and VoiceOver.
3. Test Retina/mixed-DPI/multiple displays and screen-edge positioning,
   especially end alignment and flipped-view handling.
4. Open Manage/History, switch external apps while tracking, and activate from
   the nonactivating panel. Verify no focus theft and that the key-window check
   does not suppress legitimate clicks. No activation workaround was added.
5. Repeat open/cancel/select and cross-menu cycles, profile native lifetimes,
   and inspect application channel/resource/handler counts. Include navigation,
   window disappearance and native errors. Source ownership is not a heap profile.
6. Exercise Japanese/Chinese/Korean candidate Enter/Escape, 229 event ordering,
   held keys and native Command+A/C/V/X/Z editing shortcuts.
