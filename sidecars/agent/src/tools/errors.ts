/**
 * Typed errors for tool clients. Tool failures are surfaced to the model as
 * tool results (`isError: true`) — never thrown across the agent loop — so the
 * research can continue after a failed search/scrape.
 *
 * Messages must never contain secrets (API keys are only ever named by their
 * env var, never echoed).
 */

export type ToolErrorCode =
  | "missing_api_key"
  | "http_error"
  | "timeout"
  | "network_error"
  | "invalid_response"
  /** The model's tool arguments failed the tool's schema (or an unsupported URL scheme). */
  | "invalid_arguments"
  | "document_not_allowed"
  | "document_timeout"
  | "document_error"
  | "cancelled";

export class ToolError extends Error {
  constructor(
    public readonly code: ToolErrorCode,
    message: string,
    public readonly status?: number,
  ) {
    super(message);
    this.name = "ToolError";
  }
}

export function toToolError(err: unknown, timeoutHint: string): ToolError {
  if (err instanceof ToolError) return err;
  if (err instanceof Error) {
    // Both undici/Bun fetch abort ("AbortError") and AbortSignal.timeout
    // ("TimeoutError") surface as DOMException-like errors.
    if (err.name === "TimeoutError" || err.name === "AbortError") {
      return new ToolError("timeout", timeoutHint);
    }
    return new ToolError("network_error", err.message);
  }
  return new ToolError("network_error", String(err));
}

export type FetchLike = (input: string | URL, init?: RequestInit) => Promise<Response>;

export interface JsonRequestOptions {
  url: string;
  headers: Record<string, string>;
  body: unknown;
  timeoutMs: number;
  fetchImpl?: FetchLike;
  /** Human-readable label used in error messages (e.g. "exa search"). */
  label: string;
}

/** POST JSON with a hard timeout; returns the parsed JSON body. */
export async function postJson(options: JsonRequestOptions): Promise<unknown> {
  const { url, headers, body, timeoutMs, label } = options;
  const fetchImpl = options.fetchImpl ?? fetch;
  let response: Response;
  try {
    response = await fetchImpl(url, {
      method: "POST",
      headers: { "content-type": "application/json", ...headers },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(timeoutMs),
    });
  } catch (err) {
    throw toToolError(err, `${label} timed out after ${timeoutMs}ms`);
  }
  if (!response.ok) {
    let detail = "";
    try {
      detail = (await response.text()).slice(0, 300);
    } catch {
      // ignore body read failures
    }
    throw new ToolError(
      "http_error",
      `${label} failed with HTTP ${response.status}${detail ? `: ${detail}` : ""}`,
      response.status,
    );
  }
  try {
    return (await response.json()) as unknown;
  } catch {
    throw new ToolError("invalid_response", `${label} returned a non-JSON body`);
  }
}
