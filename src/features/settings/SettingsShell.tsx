import {
  Bug,
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
  type LucideIcon,
} from "lucide-react";
import { lazy, Suspense, useMemo, useState } from "react";

import { Spinner } from "@/components/ui/Spinner";
import { cn } from "@/lib/utils/cn";
import { isSettingsTab, SettingsNavContext, type SettingsTab } from "./settings-nav";

const PAGES: Record<SettingsTab, ReturnType<typeof lazy>> = {
  general: lazy(() => import("./tabs/GeneralTab")),
  modes: lazy(() => import("./tabs/ModesTab")),
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
  { id: "modes", label: "Modes", icon: LayoutGrid },
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
  const [tab, setTab] = useState<SettingsTab>(initialTab);
  const nav = useMemo(() => ({ tab, setTab }), [tab]);
  const Page = PAGES[tab];

  return (
    <SettingsNavContext.Provider value={nav}>
      <div className="flex h-screen flex-col bg-bg text-fg">
        {/* Native overlay title bar: we draw the centered title ourselves. */}
        <div data-tauri-drag-region className="flex h-11 shrink-0 items-center justify-center">
          <span className="text-[13px] font-semibold text-fg">Bluey Settings</span>
        </div>

        <nav aria-label="Settings sections" className="flex shrink-0 justify-center border-b border-border px-3 pb-3">
          <div className="flex items-center gap-1.5 overflow-x-auto">
            {TABS.map(({ id, label, icon: Icon }) => {
              const active = id === tab;
              return (
                <button
                  key={id}
                  type="button"
                  role="tab"
                  aria-selected={active}
                  onClick={() => setTab(id)}
                  className={cn(
                    "flex h-14 w-[68px] shrink-0 flex-col items-center justify-center gap-1.5 rounded-card outline-none transition-colors",
                    active ? "bg-bg-tile text-fg" : "text-fg-muted hover:text-fg",
                  )}
                >
                  <Icon className="size-[18px]" strokeWidth={1.8} aria-hidden />
                  <span className="text-[12px] font-medium leading-none">{label}</span>
                </button>
              );
            })}
          </div>
        </nav>

        <main className={cn("min-h-0 flex-1", tab === "modes" || tab === "sessions" ? "flex" : "overflow-y-auto")}>
          <Suspense
            fallback={
              <div className="flex h-40 w-full items-center justify-center">
                <Spinner size={18} />
              </div>
            }
          >
            {tab === "modes" || tab === "sessions" ? (
              <Page />
            ) : (
              <div className="mx-auto w-full max-w-[800px] px-6 pb-16">
                <Page />
              </div>
            )}
          </Suspense>
        </main>
      </div>
    </SettingsNavContext.Provider>
  );
}
