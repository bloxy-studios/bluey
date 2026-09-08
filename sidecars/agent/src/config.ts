/**
 * Environment configuration for the agent sidecar.
 *
 * Credentials are injected by the Rust backend as process environment
 * variables (read from the macOS keychain). They must never be echoed back on
 * the protocol — error messages may name the VARIABLE, never its value.
 *
 * Two ways to reach Claude:
 *  - Anthropic direct (default): `ANTHROPIC_API_KEY` → api.anthropic.com.
 *  - Microsoft Foundry: `CLAUDE_CODE_USE_FOUNDRY=1` plus a resource name or
 *    base URL and a Foundry credential. Claude Code (which the Agent SDK drives)
 *    then talks to `https://{resource}.services.ai.azure.com/anthropic` and
 *    `model` values are Foundry *deployment names*.
 */

export const DEFAULT_RESEARCH_MODEL = "claude-sonnet-5";
export const DEFAULT_MAX_TURNS = 12;
const MAX_TURNS_CEILING = 64;

/**
 * Microsoft Foundry routing for Claude Code. Mirrors the variables the CLI
 * reads (see https://code.claude.com/docs/en/microsoft-foundry).
 */
export interface FoundryConfig {
  /** Resource name → `https://{resource}.services.ai.azure.com/anthropic`. */
  resource?: string;
  /** Full base URL; mutually exclusive with `resource` (resource wins). */
  baseUrl?: string;
  /** `ANTHROPIC_FOUNDRY_API_KEY` (falls back to `AZURE_FOUNDRY_API_KEY`). */
  apiKey?: string;
  /** `ANTHROPIC_FOUNDRY_AUTH_TOKEN` — Entra bearer token (takes precedence over the key). */
  authToken?: string;
  /** Pinned deployment names for the `opus` / `sonnet` / `haiku` aliases. */
  opusModel?: string;
  sonnetModel?: string;
  haikuModel?: string;
}

export interface AgentConfig {
  anthropicApiKey?: string;
  /** Set when `CLAUDE_CODE_USE_FOUNDRY` is truthy: route Claude through Microsoft Foundry. */
  foundry?: FoundryConfig;
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

function truthy(value: string | undefined): boolean {
  const v = value?.trim().toLowerCase();
  return v === "1" || v === "true";
}

/**
 * Derive the Foundry resource name from an Azure endpoint such as
 * `https://my-resource.openai.azure.com` or
 * `https://my-resource.services.ai.azure.com/api/projects/p` → `my-resource`.
 * Only custom-subdomain Azure hosts qualify; anything else yields `undefined`.
 */
export function foundryResourceFromEndpoint(endpoint: string | undefined): string | undefined {
  const raw = nonEmpty(endpoint);
  if (!raw) return undefined;
  let host: string;
  try {
    host = new URL(raw.includes("://") ? raw : `https://${raw}`).hostname.toLowerCase();
  } catch {
    return undefined;
  }
  const suffixes = [".openai.azure.com", ".services.ai.azure.com", ".cognitiveservices.azure.com"];
  for (const suffix of suffixes) {
    if (host.endsWith(suffix)) {
      const name = host.slice(0, -suffix.length);
      return name && !name.includes(".") ? name : undefined;
    }
  }
  return undefined;
}

function loadFoundryConfig(env: Record<string, string | undefined>): FoundryConfig | undefined {
  if (!truthy(env["CLAUDE_CODE_USE_FOUNDRY"])) return undefined;

  const explicitResource = nonEmpty(env["ANTHROPIC_FOUNDRY_RESOURCE"]);
  const explicitBaseUrl = nonEmpty(env["ANTHROPIC_FOUNDRY_BASE_URL"]);
  // Bluey already carries the Foundry resource endpoint for the OpenAI-compatible
  // path; the same resource serves Claude, so fall back to it when the
  // Claude-specific variables are absent.
  const resource =
    explicitResource ??
    (explicitBaseUrl ? undefined : foundryResourceFromEndpoint(env["AZURE_FOUNDRY_ENDPOINT"]));

  return {
    resource,
    // Claude Code rejects having both; the resource name wins.
    baseUrl: resource ? undefined : explicitBaseUrl,
    apiKey: nonEmpty(env["ANTHROPIC_FOUNDRY_API_KEY"]) ?? nonEmpty(env["AZURE_FOUNDRY_API_KEY"]),
    authToken: nonEmpty(env["ANTHROPIC_FOUNDRY_AUTH_TOKEN"]),
    opusModel: nonEmpty(env["ANTHROPIC_DEFAULT_OPUS_MODEL"]),
    sonnetModel: nonEmpty(env["ANTHROPIC_DEFAULT_SONNET_MODEL"]),
    haikuModel: nonEmpty(env["ANTHROPIC_DEFAULT_HAIKU_MODEL"]),
  };
}

export function loadConfig(env: Record<string, string | undefined> = process.env): AgentConfig {
  const parsedTurns = Number.parseInt(env["BLUEY_AGENT_MAX_TURNS"] ?? "", 10);
  const maxTurns =
    Number.isFinite(parsedTurns) && parsedTurns > 0
      ? Math.min(parsedTurns, MAX_TURNS_CEILING)
      : DEFAULT_MAX_TURNS;

  return {
    anthropicApiKey: nonEmpty(env["ANTHROPIC_API_KEY"]),
    foundry: loadFoundryConfig(env),
    exaApiKey: nonEmpty(env["EXA_API_KEY"]),
    firecrawlApiKey: nonEmpty(env["FIRECRAWL_API_KEY"]),
    // Accept both spellings: the task contract says BLUEY_RESEARCH_MODEL, the
    // repo-wide .env.example ships BLUEY_MODEL_RESEARCH. First one wins.
    model:
      nonEmpty(env["BLUEY_RESEARCH_MODEL"]) ??
      nonEmpty(env["BLUEY_MODEL_RESEARCH"]) ??
      DEFAULT_RESEARCH_MODEL,
    maxTurns,
    mockMode: truthy(env["BLUEY_AGENT_MOCK"]),
    claudeCliOverride: nonEmpty(env["BLUEY_CLAUDE_CLI"]),
  };
}

/**
 * Configuration problems that must stop a job before the CLI is spawned.
 * `variable` names the env var to set (never a value).
 */
export type ConfigProblem =
  | { code: "missing_api_key"; variable: string; message: string }
  | { code: "invalid_configuration"; variable: string; message: string };

/** Check that the model provider can be reached with the given config. */
export function checkModelCredentials(config: AgentConfig): ConfigProblem | undefined {
  if (config.foundry) {
    if (!config.foundry.resource && !config.foundry.baseUrl) {
      return {
        code: "invalid_configuration",
        variable: "ANTHROPIC_FOUNDRY_RESOURCE",
        message:
          "CLAUDE_CODE_USE_FOUNDRY is set but neither ANTHROPIC_FOUNDRY_RESOURCE nor " +
          "ANTHROPIC_FOUNDRY_BASE_URL is set — cannot locate the Microsoft Foundry endpoint",
      };
    }
    if (!config.foundry.apiKey && !config.foundry.authToken) {
      return {
        code: "missing_api_key",
        variable: "ANTHROPIC_FOUNDRY_API_KEY",
        message:
          "ANTHROPIC_FOUNDRY_API_KEY is not set (nor AZURE_FOUNDRY_API_KEY / ANTHROPIC_FOUNDRY_AUTH_TOKEN) " +
          "— cannot run deep research on Microsoft Foundry",
      };
    }
    return undefined;
  }
  if (!config.anthropicApiKey) {
    return {
      code: "missing_api_key",
      variable: "ANTHROPIC_API_KEY",
      message: "ANTHROPIC_API_KEY is not set — cannot run deep research",
    };
  }
  return undefined;
}
