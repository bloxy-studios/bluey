import { create } from "zustand";

import { pushTrace } from "@/ai/trace";
import type { BenchReport, LatencyMetrics, LatencyTrace } from "@/lib/types";

export interface DevLogEntry {
  level: string;
  target: string;
  message: string;
  at: string;
}

const MAX_LOGS = 100;

interface DevStore {
  metrics: LatencyMetrics | null;
  logs: DevLogEntry[];
  /** The last `OVERLAY_WINDOW` fast-path traces (`ai.trace`, developer mode). */
  traces: LatencyTrace[];
  /** The last `dev_bench_fast_path` report run from the Advanced tab. */
  bench: BenchReport | null;
  setMetrics(metrics: LatencyMetrics): void;
  pushLog(entry: DevLogEntry): void;
  clearLogs(): void;
  pushTrace(trace: LatencyTrace): void;
  clearTraces(): void;
  setBench(report: BenchReport | null): void;
}

export const useDevStore = create<DevStore>((set) => ({
  metrics: null,
  logs: [],
  traces: [],
  bench: null,
  setMetrics: (metrics) => set({ metrics }),
  pushLog: (entry) => set((state) => ({ logs: [...state.logs, entry].slice(-MAX_LOGS) })),
  clearLogs: () => set({ logs: [] }),
  pushTrace: (trace) => set((state) => ({ traces: pushTrace(state.traces, trace) })),
  clearTraces: () => set({ traces: [] }),
  setBench: (bench) => set({ bench }),
}));
