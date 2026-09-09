/**
 * Compiled "lite" entrypoint for bun-darwin-arm64 — the Gemini-default build.
 *
 * Nothing is embedded: the Gemini backend is pure JS (`@google/genai`), so the
 * binary is a few MB instead of the ~250 MB Claude CLI bundle. The Claude
 * backend still works from this binary when `BLUEY_CLAUDE_CLI` points at an
 * installed CLI; without it a `RESEARCH_BACKEND=claude` job fails fast with
 * `invalid_configuration` (`buildVariant: "lite"` tells the runner there is no
 * embedded CLI to fall back to). Use the full entry (`entry-darwin-arm64.ts`)
 * to ship Claude without that dependency.
 */

import { runSidecarProcess } from "./main";

void runSidecarProcess({ buildVariant: "lite" });
