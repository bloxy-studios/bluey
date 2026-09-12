/**
 * Wires every store to the event bus and performs initial loads.
 * Called once from bootstrap (and re-callable after `eventBus.dispose()` in tests).
 */

import { useAuthStore } from "@/lib/auth/auth-store";
import { eventBus } from "@/lib/tauri/event-bus";
import type { Unlisten } from "@/lib/tauri/transport";
import { useAccountsStore } from "./accountsStore";
import { useAppStore } from "./appStore";
import { useChatStore } from "./chatStore";
import { useDevStore } from "./devStore";
import { startErrorSurface } from "./errorSurface";
import { resetHudUiForTest } from "./hudUiStore";
import { useModesStore } from "./modesStore";
import { usePanelStore } from "./panelStore";
import { usePermissionsStore } from "./permissionsStore";
import { resetProactiveForTest, startProactiveLoop } from "./proactive";
import { useResearchStore } from "./researchStore";
import { useSessionStore } from "./sessionStore";
import { useSettingsStore } from "./settingsStore";
import { useTranscriptStore } from "./transcriptStore";
import { useUpdatesStore } from "./updatesStore";

let disposers: Unlisten[] = [];

export function disposeStores(): void {
  disposers.forEach((dispose) => dispose());
  disposers = [];
}

export async function initStores(): Promise<void> {
  disposeStores();

  disposers = [
    eventBus.on("app.state", (status) => useAppStore.getState().setStatus(status)),
    eventBus.on("settings.changed", (settings) => useSettingsStore.getState().applyRemote(settings)),
    eventBus.on("modes.changed", (modes) => useModesStore.getState().applyRemote(modes)),
    eventBus.on("permissions.changed", (p) => usePermissionsStore.getState().applyRemote(p)),
    eventBus.on("auth.changed", (status) => useAuthStore.getState().applyStatus(status)),
    eventBus.on("accounts.changed", (account) => useAccountsStore.getState().applyAccount(account)),
    eventBus.on("accounts.catalog", (catalog) => useAccountsStore.getState().applyCatalog(catalog)),
    eventBus.on("update.status", (status) => useUpdatesStore.getState().applyRemote(status)),
    eventBus.on("panel.state", (state) => usePanelStore.getState().applyRemote(state)),

    eventBus.on("session.started", (session) => useSessionStore.getState().setActive(session)),
    eventBus.on("session.paused", (session) => useSessionStore.getState().setActive(session)),
    eventBus.on("session.resumed", (session) => useSessionStore.getState().setActive(session)),
    eventBus.on("session.ended", () => useSessionStore.getState().setActive(null)),
    eventBus.on("session.event", (event) => useSessionStore.getState().pushEvent(event)),

    eventBus.on("transcript.partial", (segment) => useTranscriptStore.getState().applyPartial(segment)),
    eventBus.on("transcript.final", (segment) => useTranscriptStore.getState().applyFinal(segment)),
    eventBus.on("transcript.cleared", ({ sessionId }) => useTranscriptStore.getState().clear(sessionId)),
    eventBus.on("question.detected", (event) => useTranscriptStore.getState().pushQuestion(event)),
    eventBus.on("audio.level", (levels) => useTranscriptStore.getState().setLevels(levels)),

    eventBus.on("response.prepared", (response) => useChatStore.getState().setPrepared(response)),
    eventBus.on("research.event", (event) => useResearchStore.getState().apply(event)),

    eventBus.on("dev.metrics", (metrics) => useDevStore.getState().setMetrics(metrics)),
    eventBus.on("ai.trace", (trace) => useDevStore.getState().pushTrace(trace)),
    eventBus.on("dev.log", (entry) => useDevStore.getState().pushLog(entry)),

    // Cross-cutting loops: proactive preparation (HUD only) and global error toasts.
    startProactiveLoop(),
    startErrorSurface(),
  ];

  await Promise.all([
    useAuthStore.getState().load(),
    useAccountsStore.getState().load(),
    useSettingsStore.getState().load(),
    useAppStore.getState().load(),
    useModesStore.getState().load(),
    usePermissionsStore.getState().load(),
    usePanelStore.getState().load(),
    useSessionStore.getState().load(),
    useUpdatesStore.getState().load(),
  ]);
}

/** Test helper: reset all store state to its initial shape. */
export function resetStoresForTest(): void {
  useAppStore.setState({ status: null });
  useAuthStore.setState({ mode: "unconfigured", state: "unknown", user: null, signInPending: false, loaded: false });
  useSettingsStore.setState({ settings: null, lastError: null });
  useAccountsStore.setState({ accounts: [], catalogs: {}, loaded: false, lastError: null });
  useModesStore.setState({ modes: [], loaded: false });
  useSessionStore.setState({ active: null, events: [] });
  useTranscriptStore.setState({
    segments: [],
    partial: null,
    questions: [],
    levels: { microphone: 0, system: 0 },
  });
  useChatStore.setState({ turns: [], generation: 0, phase: null, activeRequestId: null, prepared: null });
  usePanelStore.setState({ state: null });
  usePermissionsStore.setState({ permissions: null });
  useDevStore.setState({ metrics: null, logs: [] });
  useResearchStore.setState({ active: null });
  useUpdatesStore.setState({ status: null, busy: false });
  resetProactiveForTest();
  resetHudUiForTest();
}
