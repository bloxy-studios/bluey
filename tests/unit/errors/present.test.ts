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
    expect(describeError(error({ kind: "storage", code: "storage.locked", message: "database is locked" })).message).toBe(
      "database is locked",
    );
    expect(describeError(error({ kind: "configuration", code: "privacy.cloud_ai_disabled" })).title).toBe("Cloud AI is off");
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
