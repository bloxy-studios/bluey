/**
 * Sidecar configuration: Anthropic-direct vs Microsoft Foundry routing, and the
 * environment handed to the Claude Code subprocess.
 */

import { describe, expect, it } from "vitest";

import { buildSubprocessEnv } from "../../sidecars/agent/src/agent";
import {
  checkClaudeCliAvailable,
  checkModelCredentials,
  DEFAULT_GEMINI_MODEL,
  DEFAULT_RESEARCH_MODEL,
  foundryResourceFromEndpoint,
  loadConfig,
  parseBackend,
} from "../../sidecars/agent/src/config";

describe("parseBackend", () => {
  it("defaults to gemini and accepts both spellings", () => {
    expect(parseBackend(undefined)).toBe("gemini");
    expect(parseBackend("")).toBe("gemini");
    expect(parseBackend("Gemini")).toBe("gemini");
    expect(parseBackend("google")).toBe("gemini");
    expect(parseBackend("claude")).toBe("claude");
    expect(parseBackend(" ANTHROPIC ")).toBe("claude");
    expect(parseBackend("unknown")).toBe("gemini");
  });
});

describe("foundryResourceFromEndpoint", () => {
  it("extracts the custom-subdomain resource name from any Foundry host", () => {
    expect(foundryResourceFromEndpoint("https://my-res.openai.azure.com")).toBe("my-res");
    expect(foundryResourceFromEndpoint("https://my-res.openai.azure.com/")).toBe("my-res");
    expect(foundryResourceFromEndpoint("https://my-res.services.ai.azure.com/api/projects/p")).toBe("my-res");
    expect(foundryResourceFromEndpoint("my-res.cognitiveservices.azure.com")).toBe("my-res");
    expect(foundryResourceFromEndpoint("HTTPS://My-Res.OpenAI.Azure.com")).toBe("my-res");
  });

  it("returns undefined for non-Foundry or malformed endpoints", () => {
    expect(foundryResourceFromEndpoint(undefined)).toBeUndefined();
    expect(foundryResourceFromEndpoint("")).toBeUndefined();
    expect(foundryResourceFromEndpoint("https://api.anthropic.com")).toBeUndefined();
    expect(foundryResourceFromEndpoint("https://eastus.api.cognitive.microsoft.com")).toBeUndefined();
    expect(foundryResourceFromEndpoint("not a url at all ://")).toBeUndefined();
  });
});

describe("loadConfig", () => {
  it("defaults to the Gemini backend with the Gemini model", () => {
    const config = loadConfig({ GEMINI_API_KEY: "AIza-test" });
    expect(config.backend).toBe("gemini");
    expect(config.geminiApiKey).toBe("AIza-test");
    expect(config.model).toBe(DEFAULT_GEMINI_MODEL);
    expect(config.foundry).toBeUndefined();
  });

  it("accepts GOOGLE_API_KEY as an alias (GEMINI_API_KEY wins)", () => {
    expect(loadConfig({ GOOGLE_API_KEY: "g" }).geminiApiKey).toBe("g");
    expect(loadConfig({ GEMINI_API_KEY: "a", GOOGLE_API_KEY: "g" }).geminiApiKey).toBe("a");
  });

  it("selects Anthropic direct with RESEARCH_BACKEND=claude and no Foundry block", () => {
    const config = loadConfig({ RESEARCH_BACKEND: "claude", ANTHROPIC_API_KEY: "sk-test" });
    expect(config.backend).toBe("claude");
    expect(config.anthropicApiKey).toBe("sk-test");
    expect(config.foundry).toBeUndefined();
    expect(config.model).toBe(DEFAULT_RESEARCH_MODEL);
  });

  it("lets BLUEY_RESEARCH_MODEL (forwarded by Rust) override the backend default", () => {
    expect(loadConfig({ BLUEY_RESEARCH_MODEL: "gemini-3.5-flash-lite" }).model).toBe("gemini-3.5-flash-lite");
    expect(loadConfig({ RESEARCH_BACKEND: "claude", BLUEY_RESEARCH_MODEL: "claude-opus-5" }).model).toBe(
      "claude-opus-5",
    );
    expect(loadConfig({ BLUEY_RESEARCH_MODEL: "   " }).model).toBe(DEFAULT_GEMINI_MODEL);
  });

  it("ignores the Bluey-side BLUEY_MODEL_RESEARCH alias (per-role override of the ACTIVE Rust provider)", () => {
    // Users set BLUEY_MODEL_RESEARCH in Bluey's .env; with RESEARCH_BACKEND=claude it may
    // name a Gemini model. Rust resolves the role for the selected backend and forwards
    // the result as BLUEY_RESEARCH_MODEL — the sidecar must never read the alias itself.
    expect(loadConfig({ RESEARCH_BACKEND: "claude", BLUEY_MODEL_RESEARCH: "gemini-3.8-flash" }).model).toBe(
      DEFAULT_RESEARCH_MODEL,
    );
    expect(loadConfig({ BLUEY_MODEL_RESEARCH: "claude-opus-5" }).model).toBe(DEFAULT_GEMINI_MODEL);
    expect(
      loadConfig({ BLUEY_RESEARCH_MODEL: "gemini-3.5-flash-lite", BLUEY_MODEL_RESEARCH: "claude-opus-5" })
        .model,
    ).toBe("gemini-3.5-flash-lite");
  });

  it("ignores Foundry variables unless CLAUDE_CODE_USE_FOUNDRY is truthy", () => {
    const config = loadConfig({
      ANTHROPIC_FOUNDRY_RESOURCE: "my-res",
      ANTHROPIC_FOUNDRY_API_KEY: "k",
    });
    expect(config.foundry).toBeUndefined();
    expect(
      loadConfig({ CLAUDE_CODE_USE_FOUNDRY: "0", ANTHROPIC_FOUNDRY_RESOURCE: "r" }).foundry,
    ).toBeUndefined();
  });

  it("reads the explicit Foundry variables", () => {
    const config = loadConfig({
      CLAUDE_CODE_USE_FOUNDRY: "1",
      ANTHROPIC_FOUNDRY_RESOURCE: " my-res ",
      ANTHROPIC_FOUNDRY_API_KEY: "foundry-key",
      ANTHROPIC_FOUNDRY_AUTH_TOKEN: "entra-token",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "claude-opus-5",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "claude-sonnet-5",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "claude-haiku-4-5",
      BLUEY_RESEARCH_MODEL: "claude-opus-5",
    });
    expect(config.foundry).toEqual({
      resource: "my-res",
      baseUrl: undefined,
      apiKey: "foundry-key",
      authToken: "entra-token",
      opusModel: "claude-opus-5",
      sonnetModel: "claude-sonnet-5",
      haikuModel: "claude-haiku-4-5",
    });
    expect(config.model).toBe("claude-opus-5");
  });

  it("prefers the resource name when both resource and base URL are given", () => {
    const config = loadConfig({
      CLAUDE_CODE_USE_FOUNDRY: "true",
      ANTHROPIC_FOUNDRY_RESOURCE: "my-res",
      ANTHROPIC_FOUNDRY_BASE_URL: "https://my-res.services.ai.azure.com/anthropic",
    });
    expect(config.foundry?.resource).toBe("my-res");
    expect(config.foundry?.baseUrl).toBeUndefined();
  });

  it("uses the base URL when no resource name is available", () => {
    const config = loadConfig({
      CLAUDE_CODE_USE_FOUNDRY: "1",
      ANTHROPIC_FOUNDRY_BASE_URL: "https://private.example.com/anthropic",
      AZURE_FOUNDRY_ENDPOINT: "https://other.openai.azure.com",
    });
    expect(config.foundry?.resource).toBeUndefined();
    expect(config.foundry?.baseUrl).toBe("https://private.example.com/anthropic");
  });

  it("falls back to the shared AZURE_FOUNDRY_* resource and key", () => {
    const config = loadConfig({
      CLAUDE_CODE_USE_FOUNDRY: "1",
      AZURE_FOUNDRY_ENDPOINT: "https://shared-res.openai.azure.com",
      AZURE_FOUNDRY_API_KEY: "shared-key",
    });
    expect(config.foundry?.resource).toBe("shared-res");
    expect(config.foundry?.apiKey).toBe("shared-key");
    expect(config.foundry?.baseUrl).toBeUndefined();
  });
});

describe("checkModelCredentials", () => {
  const claude = { RESEARCH_BACKEND: "claude" };

  it("requires GEMINI_API_KEY for the Gemini backend (default)", () => {
    expect(checkModelCredentials(loadConfig({}))).toMatchObject({
      code: "missing_api_key",
      variable: "GEMINI_API_KEY",
    });
    // A Claude key is not a Gemini credential.
    expect(checkModelCredentials(loadConfig({ ANTHROPIC_API_KEY: "k" }))).toMatchObject({
      variable: "GEMINI_API_KEY",
    });
    expect(checkModelCredentials(loadConfig({ GEMINI_API_KEY: "k" }))).toBeUndefined();
    expect(checkModelCredentials(loadConfig({ GOOGLE_API_KEY: "k" }))).toBeUndefined();
  });

  it("requires ANTHROPIC_API_KEY for Anthropic direct", () => {
    expect(checkModelCredentials(loadConfig({ ...claude }))).toMatchObject({
      code: "missing_api_key",
      variable: "ANTHROPIC_API_KEY",
    });
    expect(checkModelCredentials(loadConfig({ ...claude, ANTHROPIC_API_KEY: "k" }))).toBeUndefined();
  });

  it("requires an endpoint and a credential for Foundry", () => {
    expect(checkModelCredentials(loadConfig({ ...claude, CLAUDE_CODE_USE_FOUNDRY: "1" }))).toMatchObject({
      code: "invalid_configuration",
      variable: "ANTHROPIC_FOUNDRY_RESOURCE",
    });
    expect(
      checkModelCredentials(
        loadConfig({ ...claude, CLAUDE_CODE_USE_FOUNDRY: "1", ANTHROPIC_FOUNDRY_RESOURCE: "r" }),
      ),
    ).toMatchObject({ code: "missing_api_key", variable: "ANTHROPIC_FOUNDRY_API_KEY" });
    expect(
      checkModelCredentials(
        loadConfig({
          ...claude,
          CLAUDE_CODE_USE_FOUNDRY: "1",
          ANTHROPIC_FOUNDRY_RESOURCE: "r",
          ANTHROPIC_FOUNDRY_API_KEY: "k",
        }),
      ),
    ).toBeUndefined();
    expect(
      checkModelCredentials(
        loadConfig({
          ...claude,
          CLAUDE_CODE_USE_FOUNDRY: "1",
          ANTHROPIC_FOUNDRY_RESOURCE: "r",
          ANTHROPIC_FOUNDRY_AUTH_TOKEN: "t",
        }),
      ),
    ).toBeUndefined();
  });

  it("does not need ANTHROPIC_API_KEY when Foundry is configured", () => {
    const config = loadConfig({
      ...claude,
      CLAUDE_CODE_USE_FOUNDRY: "1",
      ANTHROPIC_FOUNDRY_RESOURCE: "r",
      ANTHROPIC_FOUNDRY_API_KEY: "k",
    });
    expect(config.anthropicApiKey).toBeUndefined();
    expect(checkModelCredentials(config)).toBeUndefined();
  });
});

describe("checkClaudeCliAvailable (lite build pre-flight)", () => {
  const claude = loadConfig({ RESEARCH_BACKEND: "claude", ANTHROPIC_API_KEY: "sk-secret" });

  it("fails a lite build's Claude job with invalid_configuration naming BLUEY_CLAUDE_CLI and the full build", () => {
    const problem = checkClaudeCliAvailable(claude, { variant: "lite" });
    expect(problem).toMatchObject({ code: "invalid_configuration", variable: "BLUEY_CLAUDE_CLI" });
    expect(problem?.message).toContain("BLUEY_CLAUDE_CLI");
    expect(problem?.message).toContain("BLUEY_AGENT_VARIANT=full");
    expect(problem?.message).not.toContain("sk-secret");
  });

  it("passes when a CLI is reachable: BLUEY_CLAUDE_CLI override, embedded binary (full build) or an un-bundled dev run", () => {
    const withOverride = loadConfig({
      RESEARCH_BACKEND: "claude",
      ANTHROPIC_API_KEY: "k",
      BLUEY_CLAUDE_CLI: "/opt/claude/claude",
    });
    expect(checkClaudeCliAvailable(withOverride, { variant: "lite" })).toBeUndefined();
    expect(
      checkClaudeCliAvailable(claude, { variant: "full", embeddedClaudePath: "/$bunfs/root/claude" }),
    ).toBeUndefined();
    // No variant → `bun src/main.ts` in dev, where the SDK finds the CLI in node_modules.
    expect(checkClaudeCliAvailable(claude, {})).toBeUndefined();
  });

  it("never applies to the Gemini backend", () => {
    expect(checkClaudeCliAvailable(loadConfig({ GEMINI_API_KEY: "k" }), { variant: "lite" })).toBeUndefined();
  });
});

describe("buildSubprocessEnv", () => {
  const baseEnv = {
    PATH: "/usr/bin",
    HOME: "/Users/test",
    // Stale provider selection that must never leak into the child.
    ANTHROPIC_BASE_URL: "https://res.services.ai.azure.com/api/projects/proj",
    ANTHROPIC_API_KEY: "stale-anthropic",
    ANTHROPIC_FOUNDRY_BASE_URL: "https://stale.example.com/anthropic",
  };

  it("injects only ANTHROPIC_API_KEY for Anthropic direct and clears stray routing", () => {
    const env = buildSubprocessEnv(
      { ...baseEnv, GEMINI_API_KEY: "AIza-secret", GOOGLE_API_KEY: "AIza-alias" },
      loadConfig({ RESEARCH_BACKEND: "claude", ANTHROPIC_API_KEY: "sk-live" }),
    );
    expect(env["PATH"]).toBe("/usr/bin");
    expect(env["HOME"]).toBe("/Users/test");
    expect(env["CLAUDE_AGENT_SDK_CLIENT_APP"]).toBe("bluey-agent/0.1.0");
    expect(env["ANTHROPIC_API_KEY"]).toBe("sk-live");
    expect(env["ANTHROPIC_BASE_URL"]).toBeUndefined();
    expect(env["ANTHROPIC_FOUNDRY_BASE_URL"]).toBeUndefined();
    expect(env["CLAUDE_CODE_USE_FOUNDRY"]).toBeUndefined();
    // Gemini credentials never reach the Claude Code subprocess.
    expect(env["GEMINI_API_KEY"]).toBeUndefined();
    expect(env["GOOGLE_API_KEY"]).toBeUndefined();
  });

  it("switches Claude Code to Microsoft Foundry with the resource name and pinned deployments", () => {
    const env = buildSubprocessEnv(
      baseEnv,
      loadConfig({
        CLAUDE_CODE_USE_FOUNDRY: "1",
        ANTHROPIC_FOUNDRY_RESOURCE: "my-res",
        ANTHROPIC_FOUNDRY_API_KEY: "foundry-key",
        ANTHROPIC_DEFAULT_OPUS_MODEL: "claude-opus-5",
        ANTHROPIC_DEFAULT_SONNET_MODEL: "claude-sonnet-5",
        ANTHROPIC_DEFAULT_HAIKU_MODEL: "claude-haiku-4-5",
      }),
    );
    expect(env["CLAUDE_CODE_USE_FOUNDRY"]).toBe("1");
    expect(env["ANTHROPIC_FOUNDRY_RESOURCE"]).toBe("my-res");
    expect(env["ANTHROPIC_FOUNDRY_API_KEY"]).toBe("foundry-key");
    expect(env["ANTHROPIC_DEFAULT_OPUS_MODEL"]).toBe("claude-opus-5");
    expect(env["ANTHROPIC_DEFAULT_SONNET_MODEL"]).toBe("claude-sonnet-5");
    expect(env["ANTHROPIC_DEFAULT_HAIKU_MODEL"]).toBe("claude-haiku-4-5");
    // Mutually exclusive with the resource name; also no Anthropic-direct leftovers.
    expect(env["ANTHROPIC_FOUNDRY_BASE_URL"]).toBeUndefined();
    expect(env["ANTHROPIC_BASE_URL"]).toBeUndefined();
    expect(env["ANTHROPIC_API_KEY"]).toBeUndefined();
    expect(env["PATH"]).toBe("/usr/bin");
  });

  it("uses the base URL form when that is all the config has", () => {
    const env = buildSubprocessEnv(
      {},
      loadConfig({
        CLAUDE_CODE_USE_FOUNDRY: "1",
        ANTHROPIC_FOUNDRY_BASE_URL: "https://private.example.com/anthropic",
        ANTHROPIC_FOUNDRY_AUTH_TOKEN: "entra",
      }),
    );
    expect(env["ANTHROPIC_FOUNDRY_BASE_URL"]).toBe("https://private.example.com/anthropic");
    expect(env["ANTHROPIC_FOUNDRY_RESOURCE"]).toBeUndefined();
    expect(env["ANTHROPIC_FOUNDRY_AUTH_TOKEN"]).toBe("entra");
    expect(env["ANTHROPIC_FOUNDRY_API_KEY"]).toBeUndefined();
  });
});
