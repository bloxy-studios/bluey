/**
 * Compiled entrypoint for bun-darwin-x64. See entry-darwin-arm64.ts for why
 * there is one entry per target (static per-arch binary embedding).
 */

import embeddedClaude from "@anthropic-ai/claude-agent-sdk-darwin-x64/claude" with { type: "file" };

import { runSidecarProcess } from "./main";

void runSidecarProcess({ embeddedClaudePath: embeddedClaude, buildVariant: "full" });
