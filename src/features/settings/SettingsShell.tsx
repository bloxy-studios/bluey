import * as Tabs from "@radix-ui/react-tabs";
import {
  Bug,
  ChevronLeft,
  ChevronRight,
  FolderOpen,
  Image,
  Info,
  Keyboard,
  LayoutGrid,
  Mic,
  Settings,
  ShieldCheck,
  Sparkles,
  History,
  UserRound,
  Lock,
  Palette,
  type LucideIcon,
} from "lucide-react";
import { lazy, Suspense, useCallback, useId, useLayoutEffect, useMemo, useRef, useState } from "react";

import { Spinner } from "@/components/ui/Spinner";
import { cn } from "@/lib/utils/cn";
import { isSettingsTab, SettingsNavContext, type SettingsTab } from "./settings-nav";
import { useSettingsTabScroll } from "./use-settings-tab-scroll";

const PAGES: Record<SettingsTab, ReturnType<typeof lazy>> = {
  general: lazy(() => import("./tabs/GeneralTab")),
  appearance: lazy(() => import("./tabs/AppearanceTab")),
  modes: lazy(() => import("./tabs/ModesTab")),
  context: lazy(() => import("./tabs/ContextTab")),
  keybinds: lazy(() => import("./tabs/KeybindsTab")),
  audio: lazy(() => import("./tabs/AudioTab")),
  screen: lazy(() => import("./tabs/ScreenTab")),
  ai: lazy(() => import("./tabs/AITab")),
  privacy: lazy(() => import("./tabs/PrivacyTab")),
  permissions: lazy(() => import("./tabs/PermissionsTab")),
  sessions: lazy(() => import("./tabs/SessionsTab")),
  profile: lazy(() => import("./tabs/ProfileTab")),
  advanced: lazy(() => import("./tabs/AdvancedTab")),
  about: lazy(() => import("./tabs/AboutTab")),
};

const TABS: Array<{ id: SettingsTab; label: string; icon: LucideIcon }> = [
  { id: "general", label: "General", icon: Settings },
  { id: "appearance", label: "Appearance", icon: Palette },
  { id: "modes", label: "Modes", icon: LayoutGrid },
  { id: "context", label: "Context", icon: FolderOpen },
  { id: "keybinds", label: "Keybinds", icon: Keyboard },
  { id: "audio", label: "Audio", icon: Mic },
  { id: "screen", label: "Screen", icon: Image },
  { id: "ai", label: "AI", icon: Sparkles },
  { id: "privacy", label: "Privacy", icon: Lock },
  { id: "permissions", label: "Permissions", icon: ShieldCheck },
  { id: "sessions", label: "Sessions", icon: History },
  { id: "profile", label: "Profile", icon: UserRound },
  { id: "advanced", label: "Advanced", icon: Bug },
  { id: "about", label: "About", icon: Info },
];

function initialTab(): SettingsTab {
  const requested = new URLSearchParams(window.location.search).get("tab");
  return isSettingsTab(requested) ? requested : "general";
}

/** Settings window: centered title drag region, icon-over-label tab bar, page. */
export function SettingsShell() {
  const [tab, setSelectedTab] = useState<SettingsTab>(initialTab);
  const mainRef = useRef<HTMLElement>(null);
  const focusNextPanel = useRef(false);
  const setTab = useCallback((next: SettingsTab) => {
    // Links within a page unmount their focused source. Keep focus in the page
    // region, but never steal it from a tab that was activated in the toolbar.
    focusNextPanel.current = mainRef.current?.contains(document.activeElement) ?? false;
    setSelectedTab(next);
  }, []);
  const nav = useMemo(() => ({ tab, setTab }), [tab, setTab]);
  const { viewportRef, listRef, edges, reveal, scroll } = useSettingsTabScroll(tab);
  const toolbarId = useId();
  const hintId = useId();
  const overflowing = edges.earlier || edges.later;
  const Page = PAGES[tab];

  useLayoutEffect(() => {
    const panel = mainRef.current?.querySelector<HTMLElement>(`[data-settings-tab="${tab}"]`);
    if (!panel) return;
    panel.scrollTop = 0;
    panel.scrollLeft = 0;
    if (focusNextPanel.current) panel.focus({ preventScroll: true });
    focusNextPanel.current = false;
  }, [tab]);

  return (
    <SettingsNavContext.Provider value={nav}>
      <Tabs.Root
        value={tab}
        onValueChange={(value) => {
          if (isSettingsTab(value)) setTab(value);
        }}
        activationMode="manual"
        dir="ltr"
        className="flex h-screen min-w-0 flex-col overflow-hidden bg-bg text-fg"
      >
        {/* Only the title strip drags. The text must not intercept Tauri's hit target. */}
        <div data-tauri-drag-region className="flex h-11 shrink-0 items-center justify-center">
          <span className="pointer-events-none text-[13px] font-semibold text-fg">Bluey Settings</span>
        </div>

        <nav aria-label="Settings sections" className="shrink-0 border-b border-border px-3 pb-2">
          <div className="flex min-w-0 items-center justify-center gap-1">
            <button
              type="button"
              aria-label="Show earlier settings sections"
              aria-controls={toolbarId}
              disabled={!edges.earlier}
              onClick={() => scroll(-1)}
              className={cn(
                "flex size-7 shrink-0 items-center justify-center rounded-control text-fg-muted hover:bg-bg-hover hover:text-fg",
                "outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:pointer-events-none disabled:opacity-30",
                !overflowing && "invisible",
              )}
            >
              <ChevronLeft className="size-4" aria-hidden />
            </button>
            <div ref={viewportRef} className="min-w-0 overflow-x-auto overscroll-x-contain">
              <Tabs.List
                ref={listRef}
                id={toolbarId}
                aria-label="Settings sections"
                aria-describedby={hintId}
                className="flex w-max items-center gap-1.5 p-1"
              >
                {TABS.map(({ id, label, icon: Icon }) => (
                  <Tabs.Trigger
                    key={id}
                    value={id}
                    onFocus={(event) => reveal(event.currentTarget)}
                    className={cn(
                      "flex h-14 w-[76px] shrink-0 flex-col items-center justify-center gap-1.5 rounded-card outline-none transition-colors",
                      "text-fg-muted hover:bg-bg-hover hover:text-fg data-[state=active]:bg-bg-tile data-[state=active]:text-fg",
                      "focus-visible:ring-2 focus-visible:ring-accent motion-reduce:transition-none",
                    )}
                  >
                    <Icon className="size-[18px]" strokeWidth={1.8} aria-hidden />
                    <span className="whitespace-nowrap text-[12px] font-medium leading-none">{label}</span>
                  </Tabs.Trigger>
                ))}
              </Tabs.List>
            </div>
            <button
              type="button"
              aria-label="Show later settings sections"
              aria-controls={toolbarId}
              disabled={!edges.later}
              onClick={() => scroll(1)}
              className={cn(
                "flex size-7 shrink-0 items-center justify-center rounded-control text-fg-muted hover:bg-bg-hover hover:text-fg",
                "outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:pointer-events-none disabled:opacity-30",
                !overflowing && "invisible",
              )}
            >
              <ChevronRight className="size-4" aria-hidden />
            </button>
          </div>
          <p id={hintId} className="sr-only">
            Use Left and Right Arrow, Home or End to browse sections. Press Enter or Space to open one.
          </p>
        </nav>

        {/* Keep the linked panels mounted; reset only the newly selected panel's scroll. */}
        <main ref={mainRef} className="flex min-h-0 min-w-0 flex-1 overflow-hidden [&_input]:min-w-0">
          {TABS.map(({ id }) => (
            <Tabs.Content
              key={id}
              value={id}
              forceMount
              hidden={tab !== id}
              data-settings-tab={id}
              className={cn(
                "min-h-0 min-w-0 flex-1 overscroll-contain outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent",
                id === "modes" || id === "sessions" ? "flex overflow-hidden" : "overflow-y-auto",
                "data-[state=inactive]:hidden",
              )}
            >
              {/* Inactive panels keep their ARIA ids, but not their expensive page trees. */}
              {tab === id ? (
                <Suspense
                  fallback={
                    <div role="status" className="flex h-40 w-full items-center justify-center gap-2 text-fg-muted">
                      <Spinner size={18} />
                      <span className="sr-only">Loading settings section</span>
                    </div>
                  }
                >
                  {id === "modes" || id === "sessions" ? (
                    <Page />
                  ) : (
                    <div className="mx-auto w-full max-w-[800px] px-6 pb-16 pt-6">
                      <Page />
                    </div>
                  )}
                </Suspense>
              ) : null}
            </Tabs.Content>
          ))}
        </main>
      </Tabs.Root>
    </SettingsNavContext.Provider>
  );
}
