import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import { createMockAccounts } from "@/lib/tauri/mock/fixtures";

const ROOT = join(process.cwd(), "tests/fixtures/fingerprints");
const PROVIDERS = ["chatgpt", "claude", "antigravity"] as const;

/** Shapes the scrubber must have replaced before anything was committed. */
const SECRET_SHAPES: [string, RegExp][] = [
  ["Anthropic OAuth token", /sk-ant-o[ar]t01-[A-Za-z0-9_-]{8,}/],
  ["API key", /\bsk-[A-Za-z0-9_-]{20,}/],
  ["Google access token", /ya29\.[A-Za-z0-9._-]{8,}/],
  ["Google refresh token", /1\/\/[A-Za-z0-9._-]{8,}/],
  ["Google API key", /AIza[0-9A-Za-z_-]{35}/],
  ["JWT", /eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}/],
  ["e-mail address", /[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+\.[a-z]{2,}/],
  ["UUID", /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i],
  ["home directory", /\/(Users|home)\/(?!<USER>)[^/\s"]+/],
];

function committedFixtures(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) return entry === "captures" ? [] : committedFixtures(path);
    return entry.endsWith(".json") ? [path] : [];
  });
}

interface Capture {
  schema: number;
  provider: string;
  source: string;
  fingerprint: { version: string; capturedOn: string };
  request: { method: string; url: string; headers: { name: string; value: string }[] };
}

describe("fingerprint fixtures", () => {
  it("document every subscription provider with the fingerprint the mock accounts show", () => {
    const accounts = createMockAccounts();
    for (const provider of PROVIDERS) {
      const dir = join(ROOT, provider, "documented");
      const files = readdirSync(dir).filter((f) => f.endsWith(".json"));
      expect(files.length, `${provider} has documented captures`).toBeGreaterThan(0);
      const account = accounts.find((a) => a.providerId === provider);
      expect(account, `mock account for ${provider}`).toBeDefined();
      for (const file of files) {
        const capture = JSON.parse(readFileSync(join(dir, file), "utf8")) as Capture;
        expect(capture.schema).toBe(1);
        expect(capture.provider).toBe(provider);
        expect(capture.source).toBe("documented");
        expect(capture.fingerprint).toEqual({
          version: account!.fingerprintVersion,
          capturedOn: account!.fingerprintCapturedOn,
        });
        expect(capture.request.url.startsWith("https://")).toBe(true);
        expect(capture.request.headers.find((h) => h.name === "authorization")?.value).toBe(
          "Bearer <ACCESS_TOKEN>",
        );
      }
    }
  });

  it("commit no secret-shaped strings", () => {
    const files = committedFixtures(ROOT);
    expect(files.length).toBeGreaterThan(0);
    for (const file of files) {
      const text = readFileSync(file, "utf8");
      for (const [what, shape] of SECRET_SHAPES) {
        expect(text, `${file} contains a ${what}`).not.toMatch(shape);
      }
    }
  });
});
