import {
  Download,
  GitBranch,
  Globe,
  LayoutGrid,
  LogOut,
  Power,
  RefreshCw,
  RotateCcw,
  Rocket,
  Tag,
  TerminalSquare,
} from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { SectionHeader } from "@/components/ui/SectionHeader";
import { Select } from "@/components/ui/Select";
import { SettingRow } from "@/components/ui/SettingRow";
import { Switch } from "@/components/ui/Switch";
import { useDebouncedCallback } from "@/hooks/useDebouncedCallback";
import { signOutEverywhere } from "@/lib/auth/auth-actions";
import { bluey } from "@/lib/tauri/api";
import { UPDATE_CHANNELS, UPDATE_CHANNEL_LABELS, type UpdateChannel } from "@/lib/types";
import { describeUpdateStatus, primaryUpdateAction } from "@/lib/updates/describe";
import { useModesStore } from "@/stores/modesStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useUpdatesStore } from "@/stores/updatesStore";

const LANGUAGES = ["English", "Spanish", "French", "German", "Portuguese", "Japanese", "Korean", "Chinese"];

export default function GeneralTab() {
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const modes = useModesStore((s) => s.modes);
  const updateStatus = useUpdatesStore((s) => s.status);
  const updatesBusy = useUpdatesStore((s) => s.busy);
  const primary = primaryUpdateAction(updateStatus, updatesBusy);
  const runPrimaryUpdateAction = () => {
    const updates = useUpdatesStore.getState();
    if (primary.action === "check") void updates.check();
    else if (primary.action === "install") void updates.install();
    else if (primary.action === "relaunch") void updates.relaunch();
  };

  const [name, setName] = useState(settings?.general.blueyName ?? "Bluey");
  useEffect(() => {
    if (settings) setName(settings.general.blueyName);
  }, [settings]);
  const saveName = useDebouncedCallback((value: string) => {
    void update({ general: { blueyName: value } });
  }, 500);

  if (!settings) return null;
  const { general } = settings;

  return (
    <>
      <SectionHeader title="General" description="Customize how Bluey works for you" />

      <SettingRow icon={Tag} title="Bluey name" description="What your assistant calls itself.">
        <Input
          value={name}
          aria-label="Bluey name"
          onChange={(e) => {
            setName(e.target.value);
            saveName(e.target.value);
          }}
          className="w-[200px]"
        />
      </SettingRow>

      <SettingRow icon={Rocket} title="Launch Bluey at login" description="Bluey will stay closed until you open it.">
        <Switch
          aria-label="Launch Bluey at login"
          checked={general.launchAtLogin}
          onCheckedChange={(checked) => void update({ general: { launchAtLogin: checked } })}
        />
      </SettingRow>

      <SettingRow icon={LayoutGrid} title="Default mode" description="Used when no session overrides it.">
        <Select
          aria-label="Default mode"
          value={general.defaultModeId}
          onChange={(e) => void bluey.modes.setDefault({ id: e.target.value })}
          options={modes.map((m) => ({ value: m.id, label: m.name }))}
        />
      </SettingRow>

      <SettingRow icon={TerminalSquare} title="Developer mode" description="Simulations, metrics and the developer overlay.">
        <Switch
          aria-label="Developer mode"
          checked={general.developerMode}
          onCheckedChange={(checked) => void update({ general: { developerMode: checked } })}
        />
      </SettingRow>

      <SectionHeader title="Updates" description="Keep Bluey current" />

      <SettingRow
        icon={RefreshCw}
        title={`Bluey ${updateStatus?.currentVersion ?? ""}`.trim()}
        description={describeUpdateStatus(updateStatus)}
      >
        <Button disabled={primary.disabled} onClick={runPrimaryUpdateAction}>
          {primary.label}
        </Button>
      </SettingRow>

      <SettingRow
        icon={GitBranch}
        title="Update channel"
        description="Latest is the stable release. Nightly follows main every night it changes and can break."
      >
        <Select
          aria-label="Update channel"
          value={settings.updates.channel}
          onChange={(e) => void update({ updates: { channel: e.target.value as UpdateChannel } })}
          options={UPDATE_CHANNELS.map((channel) => ({ value: channel, label: UPDATE_CHANNEL_LABELS[channel] }))}
        />
      </SettingRow>

      <SettingRow
        icon={Download}
        title="Automatic updates"
        description="Download and install in the background; Bluey asks before relaunching."
      >
        <Switch
          aria-label="Automatic updates"
          checked={settings.updates.automatic}
          onCheckedChange={(checked) => void update({ updates: { automatic: checked } })}
        />
      </SettingRow>

      <SectionHeader title="Language" description="Choose how Bluey responds" />

      <SettingRow icon={Globe} title="Output language" description="Your preferred language for AI answers and notes.">
        <Select
          aria-label="Output language"
          value={general.outputLanguage}
          onChange={(e) => void update({ general: { outputLanguage: e.target.value } })}
          options={LANGUAGES.map((l) => ({ value: l, label: l }))}
        />
      </SettingRow>

      <div className="mt-10 flex items-center justify-between border-t border-border pt-4">
        <span className="text-[13px] text-fg-muted">Account and app</span>
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              void update({ general: { onboardingCompleted: false } }).then(() =>
                bluey.window.open({ label: "onboarding" }),
              );
            }}
          >
            <RotateCcw className="size-3.5" aria-hidden /> Reset onboarding
          </Button>
          <Button variant="ghost" size="sm" onClick={() => void signOutEverywhere()}>
            <LogOut className="size-3.5" aria-hidden /> Log out
          </Button>
          <Button variant="ghost" size="sm" onClick={() => void bluey.app.quit()}>
            <Power className="size-3.5" aria-hidden /> Quit
          </Button>
        </div>
      </div>
    </>
  );
}
