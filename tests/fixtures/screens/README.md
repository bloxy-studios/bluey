# Fixture screens

Stand-ins for a live capture when the fast-path bench (`bun run bench:fastpath`,
`dev_bench_fast_path`) runs where ScreenCaptureKit cannot — CI runners, a Mac without
screen permission — or when two runs must see the identical frame.

| File | What | Size |
|---|---|---|
| `general-1440.jpg.b64` | a synthetic 1440 × 900 IDE-like screen (title bar, file tree, ~40 lines of dense code text, a terminal, a status bar), JPEG q 0.8, stored as base64 text so it can travel through text-only tooling | ≈ 150 KB decoded |

The bench accepts a raw `.jpg` / `.png` or a `.b64` file (`--fixture <path>`). The
pixels are not a real screenshot — they size the pipeline (encode → base64 → IPC →
request body), not the model's answer. Recapture real screens with the app's capture
when a legibility comparison is the point (`docs/LATENCY.md › Image pipeline`).
