/**
 * bluey-agent sidecar entrypoint: stdio JSON-Lines loop.
 *
 * One research job per process — the process exits after the job completes,
 * fails, or is cancelled (see docs/AGENT_SIDECAR_PROTOCOL.md).
 *
 * `startSidecar` is exported with injectable input/output/env/deps so the
 * whole loop can be exercised in-process by tests (and by mock mode).
 */

import { createInterface } from "node:readline";

import { startResearchJob, type AgentRunDeps, type ResearchJobHandle } from "./agent";
import { loadConfig, type BuildVariant } from "./config";
import { createMockExaClient, createMockFirecrawlClient } from "./mock";
import {
  deepResearchRequestSchema,
  documentResponseSchema,
  parseRequestLine,
  ProtocolWriter,
  wireError,
  type LineSink,
  type RequestId,
} from "./protocol";

const cancelParamsSchema = deepResearchRequestSchema.pick({ jobId: true });

export interface StartSidecarOptions {
  input?: NodeJS.ReadableStream;
  output?: LineSink;
  env?: Record<string, string | undefined>;
  /** Test injection: queryFn / tool clients. */
  deps?: AgentRunDeps;
  /** Set by the compiled per-target entrypoints (full build: embedded CLI). */
  embeddedClaudePath?: string;
  /** Set by the compiled per-target entrypoints (`lite` = no embedded Claude CLI). */
  buildVariant?: BuildVariant;
  /** Default: only when reading from the real process stdin. */
  installSignalHandlers?: boolean;
}

/** Runs the JSON-Lines loop; resolves with the intended process exit code. */
export function startSidecar(options: StartSidecarOptions = {}): Promise<number> {
  const input = options.input ?? process.stdin;
  const output = options.output ?? process.stdout;
  const env = options.env ?? process.env;
  const config = loadConfig(env);
  const writer = new ProtocolWriter(output);

  const baseDeps: AgentRunDeps = {
    ...options.deps,
    embeddedClaudePath: options.deps?.embeddedClaudePath ?? options.embeddedClaudePath,
    buildVariant: options.deps?.buildVariant ?? options.buildVariant,
    env: options.deps?.env ?? env,
  };
  // Mock mode: no network — fake search/scrape clients unless a test injected its own.
  if (config.mockMode) {
    baseDeps.exaClient = baseDeps.exaClient ?? createMockExaClient();
    baseDeps.firecrawlClient = baseDeps.firecrawlClient ?? createMockFirecrawlClient();
  }

  let job: ResearchJobHandle | null = null;
  let finished = false;

  return new Promise<number>((resolve) => {
    const rl = createInterface({ input, crlfDelay: Infinity });

    const finish = (code: number): void => {
      if (finished) return;
      finished = true;
      rl.close();
      resolve(code);
    };

    const handleRun = (id: RequestId, params: unknown): void => {
      if (job) {
        writer.error(
          id,
          wireError("job_already_running", "this sidecar already ran a research job (one job per process)", "sidecar"),
        );
        return;
      }
      const parsed = deepResearchRequestSchema.safeParse(params);
      if (!parsed.success) {
        const detail = parsed.error.issues
          .map((issue) => `${issue.path.join(".") || "params"}: ${issue.message}`)
          .join("; ");
        writer.error(id, wireError("invalid_params", `invalid DeepResearchRequest — ${detail}`, "sidecar"));
        return;
      }
      writer.result(id, { accepted: true });
      job = startResearchJob(parsed.data, config, writer, baseDeps);
      void job.done.finally(() => {
        // One job per process: leave the loop once the terminal event is out.
        finish(0);
      });
    };

    const handleCancel = (id: RequestId, params: unknown): void => {
      const parsed = cancelParamsSchema.safeParse(params);
      if (!parsed.success) {
        writer.error(id, wireError("invalid_params", "research.cancel requires { jobId }", "sidecar"));
        return;
      }
      if (!job || job.jobId !== parsed.data.jobId) {
        writer.error(id, wireError("unknown_job", `no running job with id "${parsed.data.jobId}"`, "sidecar"));
        return;
      }
      writer.result(id, { cancelled: true });
      job.cancel("research.cancel received");
    };

    const handleDocumentResponse = (id: RequestId, params: unknown): void => {
      const parsed = documentResponseSchema.safeParse(params);
      if (!parsed.success) {
        // document.request expects no reply frame on success; only malformed
        // frames get an error back (debuggability).
        writer.error(id, wireError("invalid_params", "invalid document.response params", "sidecar"));
        return;
      }
      // Unknown / late request ids are ignored on purpose (the 10 s timeout
      // may already have fired and been surfaced to the model).
      job?.handleDocumentResponse(parsed.data);
    };

    rl.on("line", (line: string) => {
      const trimmed = line.trim();
      if (!trimmed) return;
      const parsed = parseRequestLine(trimmed);
      if (!parsed.ok) {
        writer.error(parsed.id, wireError("invalid_request", parsed.error, "sidecar"));
        return;
      }
      const { id, method, params } = parsed.request;
      switch (method) {
        case "research.run":
          handleRun(id, params);
          break;
        case "research.cancel":
          handleCancel(id, params);
          break;
        case "document.response":
          handleDocumentResponse(id, params);
          break;
        default:
          writer.error(id, wireError("unknown_method", `unknown method "${method}"`, "sidecar"));
          break;
      }
    });

    rl.on("close", () => {
      // stdin EOF just means "no more requests" (a shell pipe closes it right
      // away). A running job continues to completion — its own `finally`
      // finishes the loop. Cancellation happens via research.cancel or a
      // termination signal, and Tauri kills the child process on shutdown.
      if (!job) finish(0);
    });

    const installSignals = options.installSignalHandlers ?? input === process.stdin;
    if (installSignals) {
      const onSignal = (): void => {
        if (job) {
          job.cancel("termination signal");
          // Safety net if the abort never unwinds.
          const timer = setTimeout(() => finish(0), 5_000);
          (timer as { unref?: () => void }).unref?.();
        } else {
          finish(0);
        }
      };
      process.once("SIGTERM", onSignal);
      process.once("SIGINT", onSignal);
    }
  });
}

/** Flush stdout before exiting so the last protocol frames are never lost. */
async function flushStdout(): Promise<void> {
  await new Promise<void>((resolve) => {
    if (process.stdout.write("")) resolve();
    else process.stdout.once("drain", () => resolve());
  });
}

/** Process entrypoint used by main.ts (dev) and the compiled per-target entries. */
export async function runSidecarProcess(options: StartSidecarOptions = {}): Promise<never> {
  const code = await startSidecar(options);
  await flushStdout();
  process.exit(code);
}

// Bun sets import.meta.main when this file is the entrypoint (dev: `bun
// src/main.ts`). The cast keeps the file typecheckable in programs without
// bun's ImportMeta augmentation (e.g. the root tsconfig pulls this file in
// through tests/).
if ((import.meta as ImportMeta & { main?: boolean }).main === true) {
  void runSidecarProcess();
}
