/**
 * Resolution of the Claude Code CLI binary the Agent SDK spawns.
 *
 * Priority:
 *  1. BLUEY_CLAUDE_CLI env var — explicit dev/test override.
 *  2. An embedded binary passed by the compiled per-target entrypoint
 *     (src/entry-darwin-*.ts) — extracted from Bun's $bunfs virtual
 *     filesystem to a real temp path via the SDK's extractFromBunfs helper
 *     (child processes cannot exec $bunfs paths).
 *  3. undefined — the SDK auto-detects the platform binary from node_modules
 *     (`@anthropic-ai/claude-agent-sdk-<platform>/claude`), which is what we
 *     want when running un-bundled in dev (`bun src/main.ts`).
 */

import { extractFromBunfs } from "@anthropic-ai/claude-agent-sdk/extract";

export interface CliPathOptions {
  embeddedClaudePath?: string;
  env?: Record<string, string | undefined>;
}

export function resolveClaudeCliPath(options: CliPathOptions = {}): string | undefined {
  const env = options.env ?? process.env;
  const override = env["BLUEY_CLAUDE_CLI"]?.trim();
  if (override) return override;
  if (options.embeddedClaudePath) {
    return extractFromBunfs(options.embeddedClaudePath);
  }
  return undefined;
}
