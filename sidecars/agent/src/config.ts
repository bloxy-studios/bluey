/**
 * Environment configuration for the agent sidecar.
 *
 * Credentials are injected by the Rust backend as process environment
 * variables (read from the macOS keychain). They must never be echoed back on
 * the protocol — error messages may name the VARIABLE, never its value.
 */

export const DEFAULT_RESEARCH_MODEL = "claude-sonnet-5";
export const DEFAULT_MAX_TURNS = 12;
const MAX_TURNS_CEILING = 64;

export interface AgentConfig {
  anthropicApiKey?: string;
  exaApiKey?: string;
  firecrawlApiKey?: string;
  /** Model used for the research agent (request.model overrides this). */
  model: string;
  /** Default max agent turns (request.maxTurns overrides this). */
  maxTurns: number;
  /** BLUEY_AGENT_MOCK=1 — run the whole job against a fake query()/tool set. */
  mockMode: boolean;
  /** BLUEY_CLAUDE_CLI — explicit path to the claude CLI binary (dev override). */
  claudeCliOverride?: string;
}

function nonEmpty(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}

export function loadConfig(env: Record<string, string | undefined> = process.env): AgentConfig {
  const parsedTurns = Number.parseInt(env["BLUEY_AGENT_MAX_TURNS"] ?? "", 10);
  const maxTurns =
    Number.isFinite(parsedTurns) && parsedTurns > 0
      ? Math.min(parsedTurns, MAX_TURNS_CEILING)
      : DEFAULT_MAX_TURNS;

  const mockRaw = env["BLUEY_AGENT_MOCK"]?.toLowerCase();

  return {
    anthropicApiKey: nonEmpty(env["ANTHROPIC_API_KEY"]),
    exaApiKey: nonEmpty(env["EXA_API_KEY"]),
    firecrawlApiKey: nonEmpty(env["FIRECRAWL_API_KEY"]),
    // Accept both spellings: the task contract says BLUEY_RESEARCH_MODEL, the
    // repo-wide .env.example ships BLUEY_MODEL_RESEARCH. First one wins.
    model:
      nonEmpty(env["BLUEY_RESEARCH_MODEL"]) ??
      nonEmpty(env["BLUEY_MODEL_RESEARCH"]) ??
      DEFAULT_RESEARCH_MODEL,
    maxTurns,
    mockMode: mockRaw === "1" || mockRaw === "true",
    claudeCliOverride: nonEmpty(env["BLUEY_CLAUDE_CLI"]),
  };
}
