import { create } from "zustand";

import type { LatencyMetrics } from "@/lib/types";

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
  setMetrics(metrics: LatencyMetrics): void;
  pushLog(entry: DevLogEntry): void;
  clearLogs(): void;
}

export const useDevStore = create<DevStore>((set) => ({
  metrics: null,
  logs: [],
  setMetrics: (metrics) => set({ metrics }),
  pushLog: (entry) => set((state) => ({ logs: [...state.logs, entry].slice(-MAX_LOGS) })),
  clearLogs: () => set({ logs: [] }),
}));
