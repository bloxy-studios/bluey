/**
 * Compiled "lite" entrypoint for bun-darwin-arm64 — the Gemini-default build.
 *
 * Nothing is embedded: the Gemini backend is pure JS (`@google/genai`), so the
 * binary is a few MB instead of the ~250 MB Claude CLI bundle. The Claude
 * backend still works from this binary when `BLUEY_CLAUDE_CLI` points at an
 * installed CLI (or the SDK finds one in node_modules); use the full entry
 * (`entry-darwin-arm64.ts`) to ship Claude without that dependency.
 */

import { runSidecarProcess } from "./main";

void runSidecarProcess();
