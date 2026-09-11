import { describe, expect, it } from "vitest";

import { describeError, presentError } from "@/lib/errors/present";
import type { BlueyError } from "@/lib/types";

function error(partial: Partial<BlueyError> & Pick<BlueyError, "kind" | "code">): BlueyError {
  return { message: "technical detail", recoverable: true, ...partial };
}

describe("describeError", () => {
  it("explains a rejected Google AI Studio key and where to fix it", () => {
    const described = describeError(
      error({ kind: "configuration", code: "config.api_key_invalid", recovery: { type: "configure_provider" } }),
    );
    expect(described.title).toBe("API key rejected");
    expect(described.message).toContain("aistudio.google.com/apikey");
  });

  it("distinguishes daily quota from a plain rate limit and surfaces the retry delay", () => {
    const daily = describeError(
      error({ kind: "network", code: "network.http_429", details: { dailyQuota: true, quotaId: "GenerateRequestsPerDay" } }),
    );
    expect(daily.title).toBe("Daily quota reached");
    expect(daily.message).toContain("midnight Pacific");

    const burst = describeError(error({ kind: "network", code: "network.http_429", details: { retryAfterMs: 4200 } }));
    expect(burst.title).toBe("Rate limited");
    expect(burst.message).toContain("Retry in 5s.");

    const long = describeError(error({ kind: "network", code: "network.http_429", details: { retryAfterMs: 600_000 } }));
    expect(long.message).toContain("about 10 minutes");
  });

  it("turns blocked finish reasons into a refusal message", () => {
    const described = describeError(error({ kind: "ai", code: "ai.blocked_prohibited_content" }));
    expect(described.title).toBe("Answer refused");
    expect(described.message).toContain("prohibited content");
  });

  it("falls back to kind copy, then the technical message", () => {
    expect(describeError(error({ kind: "ai", code: "ai.http_418" })).message).toBe(
      "The model didn't answer. This is usually temporary.",
    );
    const locked = describeError(error({ kind: "storage", code: "storage.locked", message: "database is locked" }));
    expect(locked).toEqual({ title: "Storage problem", message: "database is locked" });
    expect(describeError(error({ kind: "configuration", code: "privacy.cloud_ai_disabled" })).title).toBe("Cloud AI is off");
  });

  it("explains import failures and points at the transcription role", () => {
    const unsupported = describeError(error({ kind: "not_supported", code: "not_supported.transcribe_file" }));
    expect(unsupported.title).toBe("Can't transcribe files with this provider");
    expect(unsupported.message).toContain("Settings → AI → Models → Transcription");
    expect(unsupported.message).toContain("Google Gemini");

    expect(describeError(error({ kind: "transcription", code: "transcription.no_speech" })).title).toBe("No speech found");
    expect(describeError(error({ kind: "ai", code: "ai.transcription_parse" })).title).toBe("Couldn't read the transcript");

    // Rust writes the specific reason for invalid parameters; the recording checks get the import title.
    expect(
      describeError(error({ kind: "internal", code: "internal.invalid_params", message: "the recording is empty" })),
    ).toEqual({ title: "Can't import this file", message: "the recording is empty" });
    expect(
      describeError(
        error({
          kind: "internal",
          code: "internal.invalid_params",
          message: 'unsupported recording format "m4a" — use WAV, MP3, AIFF, AAC, OGG or FLAC',
        }),
      ).title,
    ).toBe("Can't import this file");
    const shortcut = describeError(
      error({ kind: "internal", code: "internal.invalid_params", message: "`Q` is not a valid shortcut" }),
    );
    expect(shortcut.title).not.toBe("Can't import this file");
    expect(shortcut.message).toBe("`Q` is not a valid shortcut");
  });

  it("covers the remaining provider codes and the informational STT fallback", () => {
    expect(describeError(error({ kind: "configuration", code: "config.unknown_provider" })).title).toBe("Provider not found");
    expect(describeError(error({ kind: "configuration", code: "config.no_preset" })).title).toBe("No recommended models");
    expect(describeError(error({ kind: "audio", code: "audio.stt_fallback" }))).toEqual({
      title: "Using Apple Speech",
      message: "Cloud transcription isn't available right now, so Bluey is transcribing on-device.",
    });
  });
});

describe("presentError", () => {
  it("keeps the recovery button wiring", () => {
    const presented = presentError(
      error({ kind: "configuration", code: "config.model_not_found", recovery: { type: "configure_provider" } }),
    );
    expect(presented.title).toBe("Model not available");
    expect(presented.actionLabel).toBe("Configure provider");
    expect(typeof presented.action).toBe("function");

    const retry = presentError(error({ kind: "network", code: "network.timeout", recovery: { type: "retry" } }), {
      onRetry: () => {},
    });
    expect(retry.actionLabel).toBe("Retry");
    expect(presentError(error({ kind: "network", code: "network.timeout", recovery: { type: "retry" } })).action).toBeUndefined();
  });
});

describe("subscription accounts (ADR 0009)", () => {
  it("names what Bluey did instead for every stop signal", () => {
    const drift = describeError(error({ kind: "authentication", code: "account.fingerprint_drift" }));
    expect(drift.title).toBe("Provider stopped recognising Bluey");
    expect(drift.message).toContain("rather than bill your extra usage");
    expect(drift.message).toContain("API key");

    const extra = describeError(error({ kind: "authentication", code: "account.extra_usage_blocked" }));
    expect(extra.title).toBe("Paused to avoid extra-usage charges");

    const expired = describeError(error({ kind: "authentication", code: "account.needs_reauth" }));
    expect(expired.title).toBe("Subscription sign-in expired");

    const pending = describeError(
      error({ kind: "authentication", code: "account.provider_pending", message: "Claude sign-in is not built into this version of Bluey yet (PR 3b)." }),
    );
    expect(pending.title).toBe("Not available yet");
    expect(pending.message).toContain("PR 3b");
  });

  it("describes a rate-limited window with its reset time", () => {
    const limited = describeError(
      error({
        kind: "authentication",
        code: "account.rate_limited",
        details: { until: new Date(Date.now() + 3_600_000).toISOString(), window: "5h" },
      }),
    );
    expect(limited.title).toBe("Plan limit reached");
    expect(limited.message).toContain("5h window");
    expect(limited.message).toMatch(/resets at \d/);
    expect(limited.message).toContain("API key meanwhile");

    const noReset = describeError(error({ kind: "authentication", code: "account.rate_limited" }));
    expect(noReset.message).not.toContain("resets at");
  });

  it("offers Reconnect for an expired account and Use API key instead for an unavailable one", () => {
    const reconnect = presentError(
      error({
        kind: "authentication",
        code: "account.needs_reauth",
        recovery: { type: "reconnect_account", accountId: "claude", providerId: "claude" },
      }),
    );
    expect(reconnect.actionLabel).toBe("Reconnect");
    expect(reconnect.action).toBeTypeOf("function");

    const opened: string[] = [];
    const useKey = presentError(
      error({ kind: "authentication", code: "account.fingerprint_drift", recovery: { type: "use_api_key" } }),
      { onOpenSettingsTab: (tab) => opened.push(tab) },
    );
    expect(useKey.actionLabel).toBe("Use API key instead");
    void useKey.action?.();
    expect(opened).toEqual(["ai"]);
  });
});
