#!/usr/bin/env bun
/**
 * `bun run bench:fastpath -- [--iterations 30] [--provider mock|<id>] [--fixture <path>] [--ci] [--out <file>]`
 *
 * The ⌘↵ fast-path bench (ADR 0010 §2, `docs/LATENCY.md › The bench`). Launches
 * the app with `--features dev-tools` and the `BLUEY_BENCH_*` environment set,
 * so it boots, runs `dev_bench_fast_path`, prints the percentile table, writes
 * the JSON report and exits. `--ci` fails the run when the local total
 * (request sent) p50 is above the informative runner threshold.
 *
 * Needs a Mac that can run the app; the mock provider needs no key. Without
 * screen permission pass a fixture screen (`tests/fixtures/screens/`).
 */

import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

interface Args {
  iterations: number;
  provider: string;
  fixture?: string;
  out: string;
  ci: boolean;
  release: boolean;
}

/** CI runners are slow and noisy: informative, never a gate on network numbers. */
const CI_LOCAL_TOTAL_P50_MS = 400;

function parseArgs(argv: string[]): Args {
  const args: Args = {
    iterations: 30,
    provider: "mock",
    out: resolve(".bench", `fastpath-${new Date().toISOString().replace(/[:.]/g, "-")}.json`),
    ci: false,
    release: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const value = argv[i + 1];
    switch (flag) {
      case "--iterations":
        args.iterations = Number.parseInt(value ?? "", 10);
        i += 1;
        break;
      case "--provider":
        args.provider = value ?? "mock";
        i += 1;
        break;
      case "--fixture":
        args.fixture = value;
        i += 1;
        break;
      case "--out":
        args.out = resolve(value ?? args.out);
        i += 1;
        break;
      case "--ci":
        args.ci = true;
        break;
      case "--release":
        args.release = true;
        break;
      case "--help":
      case "-h":
        console.info(
          "bun run bench:fastpath -- [--iterations 30] [--provider mock|<id>] [--fixture <path>] [--ci] [--out <file>] [--release]",
        );
        process.exit(0);
        break;
      default:
        break;
    }
  }
  if (!Number.isFinite(args.iterations) || args.iterations < 1 || args.iterations > 200) {
    console.error("bench: --iterations must be between 1 and 200");
    process.exit(64);
  }
  if (args.fixture && !existsSync(args.fixture)) {
    console.error(`bench: fixture not found: ${args.fixture}`);
    process.exit(66);
  }
  return args;
}

interface BenchRowJson {
  stage: string;
  label: string;
  samples: number;
  p50Ms?: number;
  p95Ms?: number;
}

interface BenchReportJson {
  provider: string;
  model?: string;
  iterations: number;
  failures: number;
  rows: BenchRowJson[];
  localTotalP50Ms?: number;
  localTotalP95Ms?: number;
  markdown: string;
}

async function main(): Promise<void> {
  const args = parseArgs(process.argv.slice(2));
  mkdirSync(dirname(args.out), { recursive: true });

  const env: Record<string, string | undefined> = {
    ...process.env,
    BLUEY_BENCH_FASTPATH: "1",
    BLUEY_BENCH_ITERATIONS: String(args.iterations),
    BLUEY_BENCH_PROVIDER: args.provider,
    BLUEY_BENCH_OUT: args.out,
  };
  if (args.fixture) env.BLUEY_BENCH_FIXTURE = resolve(args.fixture);

  const cargoArgs = ["--features", "dev-tools"];
  if (args.release) cargoArgs.unshift("--release");
  console.info(
    `bench: ${args.iterations} runs on \`${args.provider}\`${args.fixture ? ` with fixture ${args.fixture}` : " (live capture)"} — launching the app…`,
  );
  const app = Bun.spawn(["bun", "run", "tauri", "dev", "--", ...cargoArgs], {
    env,
    stdout: "inherit",
    stderr: "inherit",
  });
  const code = await app.exited;

  if (!existsSync(args.out)) {
    console.error(`bench: the app left no report at ${args.out} (exit code ${code})`);
    process.exit(code === 0 ? 70 : code);
  }
  const report = JSON.parse(readFileSync(args.out, "utf8")) as BenchReportJson;
  console.info(`\nbench: report written to ${args.out}`);
  if (report.failures > 0) {
    console.error(`bench: ${report.failures} run(s) failed`);
  }
  if (args.ci) {
    const local = report.localTotalP50Ms;
    if (local === undefined) {
      console.error("bench: no local total measured");
      process.exit(1);
    }
    if (local > CI_LOCAL_TOTAL_P50_MS) {
      console.error(`bench: local total p50 ${local.toFixed(0)} ms is above the ${CI_LOCAL_TOTAL_P50_MS} ms CI threshold`);
      process.exit(1);
    }
    console.info(`bench: local total p50 ${local.toFixed(0)} ms ≤ ${CI_LOCAL_TOTAL_P50_MS} ms`);
  }
  process.exit(report.failures > 0 ? 2 : 0);
}

void main();
