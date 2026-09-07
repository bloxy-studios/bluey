/**
 * Compiled entrypoint for bun-darwin-arm64.
 *
 * Embeds the Claude Code CLI native binary from the platform optional
 * dependency into the single-file executable; at runtime it is extracted from
 * Bun's $bunfs to a real temp path (child processes cannot exec $bunfs paths)
 * and passed to the SDK as `pathToClaudeCodeExecutable`.
 *
 * `bun build --compile` requires a statically analyzable per-target import,
 * which is why there is one entry file per target instead of a single main.ts.
 */

import embeddedClaude from "@anthropic-ai/claude-agent-sdk-darwin-arm64/claude" with { type: "file" };

import { runSidecarProcess } from "./main";

void runSidecarProcess({ embeddedClaudePath: embeddedClaude });
