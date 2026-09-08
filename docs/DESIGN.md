# Bluey Visual Design Spec

Derived from the reference screenshots in `cluely-screenshorts/` (the design authority).
Bluey must feel like a native macOS utility: precise spacing, subtle translucency, restrained
typography, compact hierarchy, elegant micro-interactions. No hero cards, gradients, glassy
rainbow "AI" styling, or generic chatbot layouts.

## Tokens (Tailwind v4 `@theme` in `src/app/styles/theme.css`)

| Token | Dark (default) | Notes |
|---|---|---|
| `--color-bg` | `#0e0e0e` | settings window canvas |
| `--color-bg-elevated` | `#161616` | cards, sidebar items (selected) |
| `--color-bg-tile` | `#1e1e1e` | 48×48 icon tiles, keycaps, segmented tabs (active) |
| `--color-bg-hover` | `#1a1a1a` | row hover |
| `--color-border` | `#262626` | 1px hairlines |
| `--color-border-strong` | `#333333` | inputs focus |
| `--color-fg` | `#f2f2f2` | primary text |
| `--color-fg-muted` | `#9a9a9a` | descriptions, inactive tab labels |
| `--color-fg-subtle` | `#6b6b6b` | section labels ("Looking for work") |
| `--color-accent` | `#0a84ff` | primary buttons, links, active check badge |
| `--color-accent-hover` | `#2b95ff` | |
| `--color-accent-soft` | `#0f2a45` | "New Mode" + tile background |
| `--color-danger` | `#ff453a` | "Delete account" |
| `--color-success` | `#30d158` | listening dot |
| `--color-hud-bg` | `rgba(22,22,22,0.78)` | HUD panel over `backdrop-filter: blur(24px) saturate(140%)` |
| `--color-hud-input` | `rgba(255,255,255,0.06)` | HUD input row |
| `--color-hud-border` | `rgba(255,255,255,0.08)` | HUD hairline between rows |
| `--color-tooltip-bg` | `#0b0b0b` | tooltips |
| `--radius-panel` | `16px` | HUD |
| `--radius-card` | `12px` | cards, tiles |
| `--radius-control` | `8px` | buttons, inputs, keycaps 6px |
| Font | `-apple-system, "SF Pro Text", Inter, system-ui` | 13px base (settings), 15px response body |
| Mono | `"SF Mono", ui-monospace, Menlo` | inline code with `bg #1e1e1e`, 13px |

Light theme mirrors the same scale on `#f5f5f7` / `#ffffff` with `#1d1d1f` text. Follow
`prefers-color-scheme` when theme = system.

## Settings window (930 × 690, resizable)

* Native title bar (Overlay style, hidden title). We render our own centered title
  "Bluey Settings" in a 44px drag region (`data-tauri-drag-region`).
* **Top tab bar**: horizontal icon-over-label tabs, centered, 12px gap. Each tab is 76×56,
  icon 18px, label 12px medium. Inactive: `fg-muted`; active: `fg` on `bg-tile` rounded 12px.
  Tabs (in order): General, Modes, Keybinds, Audio, Screen, AI, Privacy, Permissions,
  Sessions, Profile, Advanced, About (Calendar/Notifications/Billing from the reference are
  intentionally omitted — Bluey has no billing). Hairline below the bar.
* **Content**: 24px horizontal padding, scrollable, max-width 800px.
  * Section header: 15px semibold + 13px muted description, 24px above, 12px below.
  * **Setting row**: 48×48 tile (`bg-tile`, radius 12, 20px line icon, `fg-muted`), 12px gap,
    title 14px medium + 13px muted description, right-aligned control. Rows are 68px tall,
    separated by 8px, no borders.
  * Controls: **Switch** (44×26 pill; off `#2a2a2a` knob white; on `accent`), **Select**
    (height 36, `bg-elevated`, border, radius 8, chevron), **Primary button** (accent, white
    13px medium, height 34, radius 8, padding 0 16), **Secondary button** (`bg-elevated`,
    border, same metrics), **Link button** (accent text).
  * **Card**: `bg-elevated`, 1px border, radius 12, 20px padding (About release notes,
    Profile).
  * Footer strip (General): left muted "Account and app", right ghost buttons with icons:
    Reset onboarding, Log out, Quit.
* **Modes tab**: two panes. Left 250px list on `bg` with 12px padding: "New Mode" row
  (tile `accent-soft` with accent `+`), "General" row (doc icon), then group label
  "Looking for work" (11px `fg-subtle`, hairline to the right) followed by mode rows
  (icon tile 36×36, name 15px). Selected row `bg-elevated` radius 12; the active mode shows
  a 20px accent circle with a white check at the right. Right pane 32px padding: title 28px
  semibold, "…" more-menu button top-right (36×36 circle `bg-tile`), label "Meeting context"
  (14px semibold) + textarea (`bg-elevated`, border, radius 10, 15px text, 160–200px tall),
  label "Files" + dashed dropzone (border `#3a3a3a` dashed, radius 12, 220px tall) with
  stacked-documents illustration, "Adding files gives more context to Bluey" (15px medium)
  and "Drag & drop files here to add them, or browse files" (13px muted, link accent). Sticky
  bottom bar with hairline and right-aligned primary button "Set Active" / disabled "Active".
* **Keybinds tab**: header "Keyboard shortcuts" + description "Bluey works with these easy to
  remember commands. Click any of the keybinds to edit." Groups General / Window / Scroll
  (15px semibold). Rows: 20px line icon, label 14px, right-aligned **keycaps**: 24×24 (wider
  for glyph pairs) `bg-tile` radius 6, 12px `fg-muted` glyphs (⌘ ⇧ ⌃ ⌥ ↵ ↑ ↓ ← → \ , R). Clicking
  a row enters recording mode (caps highlighted accent, "Press shortcut…", Esc cancels);
  conflicts show an inline warning.
* **Profile / Security**: Clerk `<UserProfile />` bundled with dark theme variables matching the
  tokens (card background `bg-elevated`).
* **AI tab**: opens with **Default provider** — one select that re-points every role at the
  chosen provider's recommended models (Gemini first; ADR 0007) — then the provider cards
  (Gemini first, "Default" accent badge on the nominated one). Each card: name + kind pill,
  Edit, enable switch, hairline, write-only key field ("Key saved ••••" / Replace) with a
  "Get a free key at aistudio.google.com/apikey" link for Gemini, a ghost
  "✦ Use recommended models" button (disabled until a key is saved) and "Test connection"
  whose failures render as the standard error banner with its recovery button. The provider
  dialog lists Gemini first, makes the base URL optional for Gemini/Anthropic and shows API
  version + deployments only for Foundry. Models rows show the provider's catalogue for that
  role and a link "Use <recommended>" when the assignment differs from the preset; an
  "Embedding size" select (768 · 1536 · 3072) appears when embeddings run on Gemini. Research:
  Web search, Deep research agent, **Research backend** (Gemini / Claude — the Anthropic agent
  key field only for Claude), Exa / Firecrawl keys.
* **Onboarding → Connect Gemini** (after Name): key field with the AI Studio link, "Gemini is
  connected" once a key is stored, "Use another provider" (opens Settings → AI) and
  "Skip for now"; Continue is enabled once a key exists (any provider) or the step is skipped.
* **About**: card with release note (title + date, bullet list), rows Help Center (Open ↗),
  Contact Support (Email ↗), Bluey Version (right-aligned version text).

## HUD (main window — NSPanel, transparent)

* Panel width 690px (user-configurable), idle height 108px, radius 16, background
  `hud-bg` + blur, 1px inner border `hud-border`, no shadow chrome beyond a soft 0 8px 32px
  rgba(0,0,0,.35). Whole first row is a drag region except the input.
* **Idle layout**: row 1 (56px): text input placeholder "Ask anything about your screen"
  (15px, `fg-muted`), right: 36×36 rounded square (`rgba(255,255,255,.1)`) with ↵ icon.
  Hairline. Row 2 (52px): left — Bluey logo (28px circle icon) then an optional blue pill
  ("Update Available" in the reference; Bluey shows the current **mode name** pill or the
  state pill `● Listening`); center — icon buttons 32×32 radius 8, 18px icons: *Screen*
  (image icon, tooltip "Uses Screen" / "Screen off"), *Visibility* (eye / eye-off, tooltip
  "Detectable" / "Content-protected"), *Mode* (grid 2×2, tooltip = mode name, opens the mode
  menu), thin vertical divider, *Audio* (waveform, tooltip "Start Audio Session" /
  "Stop Audio Session", pulsing green dot when listening); right — "New Chat" label with
  keycaps ⌘ R when a response exists, otherwise "History" label + 32×32 down-arrow button.
* **Tooltips**: `tooltip-bg`, white 13px, radius 8, 6px 10px padding, 6px above the button,
  fade/scale 120ms.
* **Mode menu**: 200px dark menu (`#141414`, radius 12, border), items 15px with 10px 16px
  padding, active mode shows a check at the right, separator, "Manage" row with grid icon
  (opens Settings → Modes).
* **Response layout** (expanded; panel grows to 380–620px, animated height 180ms
  ease-out): header row: 36px circle back button (←), "Ask follow-up" input, right: stop
  button (■ in circle) while streaming or ↵ when idle. Body scrolls: the user's prompt as a
  right-aligned grey pill ("Assist" / the question, `rgba(255,255,255,.12)` radius 12,
  padding 10px 16px), then the response in 15px/1.55 `fg` with markdown (bullets, inline
  code on `#1e1e1e`, code blocks with header bar: language + Copy). A floating 36px
  circular ↓ button appears bottom-right when not scrolled to the end. Bottom toolbar row
  stays as in idle with "New Chat ⌘ R".
* **States** (label in the left pill): Idle `Bluey · General`, Listening `● Listening`
  (green dot), Capturing `◌ Reading screen`, Thinking `◌ Thinking` (spinner glyph), Error
  `! Something went wrong` + recovery button. Streaming text appears progressively; code
  blocks are buffered until their closing fence.
* **Actions on a response**: Copy answer, Copy code, 👍 / 👎 (with "Why wasn't this
  useful?" chips: Wrong, Too long, Not relevant, Missed context, Wrong tone), Regenerate,
  Expand/Collapse solution. "Copied" confirmation for ~1s.
* Respect `prefers-reduced-motion` and the user's reducedMotion setting: disable the height
  animation, pulse and fades.

## Menu bar

Template icon only (monochrome Bluey mark). Menu: current mode (disabled label) · Start/Stop
Listening · Capture Screen · Ask Bluey · Sessions · Modes · Settings… · Pause/Resume Bluey ·
Quit Bluey. Shows a listening/capturing indicator via the item text (e.g. "● Listening").

## Onboarding window (760 × 560)

Minimal centered steps with a 6px progress dots row at the top: Welcome → Sign in (Clerk) →
Name your Bluey → Permissions (one screen per permission: What Bluey needs / Why / What it can
access, buttons Continue + Open System Settings) → Choose default mode → Configure shortcuts →
Test screen → Test microphone → Test AI → Ready. Typography: title 24px semibold, body 14px
muted, primary button bottom-right, "Back" ghost bottom-left.

## Motion

Short (120–180ms), subtle, interruptible. Animate: panel appearance (fade + 4px rise),
response arrival (fade), mode switch (pill crossfade), listening state (pulse), copy
confirmation. Nothing else.
