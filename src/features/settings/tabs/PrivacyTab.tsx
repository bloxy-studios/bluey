import { Ban, Cloud, Eye, FileText, HardDrive, History, Image as ImageIcon, Mic } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/Button";
import { ConfirmDialog } from "@/components/ui/Dialog";
import { SectionHeader } from "@/components/ui/SectionHeader";
import { Select } from "@/components/ui/Select";
import { SegmentedTabs } from "@/components/ui/Tabs";
import { SettingRow } from "@/components/ui/SettingRow";
import { Switch } from "@/components/ui/Switch";
import { showToast } from "@/components/ui/toast-store";
import { bluey } from "@/lib/tauri/api";
import type { DataUsageStats } from "@/lib/tauri/commands";
import type { CaptureProtection, DisplayMode } from "@/lib/types";
import { formatBytes } from "@/lib/utils/format";
import { useSettingsStore } from "@/stores/settingsStore";

interface DangerAction {
  id: string;
  label: string;
  description: string;
  confirmTitle: string;
  run: () => Promise<void>;
}

export default function PrivacyTab() {
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const [protection, setProtection] = useState<CaptureProtection | null>(null);
  const [stats, setStats] = useState<DataUsageStats | null>(null);
  const [confirm, setConfirm] = useState<DangerAction | null>(null);

  const refreshStats = async () => {
    try {
      setStats(await bluey.data.usageStats());
    } catch (error) {
      console.warn("[privacy] usage stats failed", error);
    }
  };

  useEffect(() => {
    void refreshStats();
    void bluey.capture.getProtection().then(setProtection).catch(() => undefined);
  }, []);

  if (!settings) return null;
  const { privacy } = settings;

  const setDisplayMode = async (mode: DisplayMode) => {
    await update({ privacy: { displayMode: mode } });
    try {
      setProtection(await bluey.capture.setProtection({ enabled: mode === "privacy" }));
    } catch (error) {
      console.warn("[privacy] setProtection failed", error);
    }
  };

  const disableAllCapture = async () => {
    try {
      await bluey.capture.observeStop();
      await bluey.audio.stop();
      await update({ screen: { observation: "manual" } });
      showToast("All capture disabled");
    } catch (error) {
      console.warn("[privacy] disable capture failed", error);
    }
  };

  const dangers: DangerAction[] = [
    {
      id: "sessions",
      label: "Delete all sessions",
      description: "Sessions, timelines, responses and summaries.",
      confirmTitle: "Delete all sessions?",
      run: async () => {
        await bluey.session.deleteAll();
        await refreshStats();
      },
    },
    {
      id: "documents",
      label: "Delete documents",
      description: "Everything in your context library.",
      confirmTitle: "Delete all documents?",
      run: async () => {
        await bluey.documents.deleteAll();
        await refreshStats();
      },
    },
    {
      id: "transcripts",
      label: "Clear transcripts",
      description: "All stored transcript segments.",
      confirmTitle: "Clear all transcripts?",
      run: async () => {
        await bluey.data.clearTranscripts();
        await refreshStats();
      },
    },
    {
      id: "screenshots",
      label: "Clear screenshots",
      description: "Cached screen captures.",
      confirmTitle: "Clear cached screenshots?",
      run: async () => {
        await bluey.data.deleteScreenshots();
        await refreshStats();
      },
    },
    {
      id: "cache",
      label: "Clear AI cache",
      description: "Cached AI responses and embeddings.",
      confirmTitle: "Clear the AI cache?",
      run: async () => {
        await bluey.data.clearAiCache();
        await refreshStats();
      },
    },
    {
      id: "reset",
      label: "Reset Bluey",
      description: "Erase everything and restore defaults.",
      confirmTitle: "Reset Bluey completely?",
      run: async () => {
        await bluey.data.resetAll();
        await refreshStats();
      },
    },
  ];

  return (
    <>
      <SectionHeader title="Privacy Center" description="What Bluey shows, stores and sends" />

      <SettingRow
        icon={Eye}
        title="Display mode"
        description={protection?.note ?? "Privacy mode hides Bluey from screen sharing where macOS supports it."}
      >
        <SegmentedTabs
          aria-label="Display mode"
          value={privacy.displayMode}
          onValueChange={(v) => void setDisplayMode(v)}
          options={[
            { value: "standard", label: "Standard" },
            { value: "privacy", label: "Privacy" },
          ]}
        />
      </SettingRow>

      <SettingRow icon={History} title="Keep session history" description="Store sessions and their timelines on this Mac.">
        <Switch aria-label="Keep session history" checked={privacy.storeSessionHistory} onCheckedChange={(v) => void update({ privacy: { storeSessionHistory: v } })} />
      </SettingRow>

      <SettingRow icon={ImageIcon} title="Keep screenshots" description="Store captured frames with sessions.">
        <Switch aria-label="Keep screenshots" checked={privacy.storeScreenshots} onCheckedChange={(v) => void update({ privacy: { storeScreenshots: v } })} />
      </SettingRow>

      <SettingRow icon={FileText} title="Keep transcripts" description="Store what was said during sessions.">
        <Switch aria-label="Keep transcripts" checked={privacy.storeTranscripts} onCheckedChange={(v) => void update({ privacy: { storeTranscripts: v } })} />
      </SettingRow>

      <SettingRow icon={Mic} title="Raw audio" description="Recordings are never kept unless you choose otherwise.">
        <Select
          aria-label="Raw audio retention"
          value={privacy.storeRawAudio}
          onChange={(e) => void update({ privacy: { storeRawAudio: e.target.value as typeof privacy.storeRawAudio } })}
          options={[
            { value: "never", label: "Never keep" },
            { value: "until_session_end", label: "Until session ends" },
            { value: "custom", label: "Custom window" },
          ]}
        />
      </SettingRow>

      <SettingRow icon={Cloud} title="Cloud AI" description="Allow sending context to your configured cloud providers.">
        <Switch aria-label="Cloud AI" checked={privacy.cloudAiEnabled} onCheckedChange={(v) => void update({ privacy: { cloudAiEnabled: v } })} />
      </SettingRow>

      <SettingRow icon={Ban} title="Disable all capture" description="One click: stop screen observation and the audio session.">
        <Button variant="secondary" onClick={() => void disableAllCapture()}>
          Disable all capture
        </Button>
      </SettingRow>

      <SectionHeader title="Data" description="Everything Bluey stores lives on this Mac" />

      {stats ? (
        <div className="mb-4 grid grid-cols-3 gap-2.5">
          {[
            { label: "Sessions", value: String(stats.sessions) },
            { label: "Responses", value: String(stats.responses) },
            { label: "Transcript segments", value: String(stats.transcriptSegments) },
            { label: "Screenshots", value: String(stats.screenshots) },
            { label: "Documents", value: String(stats.documents) },
            { label: "Database", value: formatBytes(stats.dbSizeBytes) },
          ].map((item) => (
            <div key={item.label} className="rounded-card border border-border bg-bg-elevated px-3.5 py-3">
              <div className="text-[18px] font-semibold text-fg">{item.value}</div>
              <div className="text-[12px] text-fg-muted">{item.label}</div>
            </div>
          ))}
        </div>
      ) : null}

      <div className="flex flex-col">
        {dangers.map((action) => (
          <SettingRow key={action.id} icon={HardDrive} title={action.label} description={action.description}>
            <Button variant={action.id === "reset" ? "danger" : "secondary"} size="sm" onClick={() => setConfirm(action)}>
              {action.label}
            </Button>
          </SettingRow>
        ))}
      </div>

      {confirm ? (
        <ConfirmDialog
          open
          onOpenChange={(open) => {
            if (!open) setConfirm(null);
          }}
          title={confirm.confirmTitle}
          description={`${confirm.description} This cannot be undone.`}
          confirmLabel={confirm.label}
          onConfirm={async () => {
            await confirm.run();
            showToast("Done");
          }}
        />
      ) : null}
    </>
  );
}
