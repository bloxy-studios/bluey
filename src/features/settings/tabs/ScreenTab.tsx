import { Eye, Languages, Monitor, ScanText, Timer, AppWindowMac } from "lucide-react";
import { useEffect, useState } from "react";

import { SectionHeader } from "@/components/ui/SectionHeader";
import { Select } from "@/components/ui/Select";
import { SettingRow } from "@/components/ui/SettingRow";
import { bluey } from "@/lib/tauri/api";
import type { DisplayInfo } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";

const OCR_LANGUAGES = [
  { value: "en-US", label: "English" },
  { value: "es-ES", label: "Spanish" },
  { value: "fr-FR", label: "French" },
  { value: "de-DE", label: "German" },
  { value: "ja-JP", label: "Japanese" },
  { value: "zh-Hans", label: "Chinese (Simplified)" },
];

export default function ScreenTab() {
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const [displays, setDisplays] = useState<DisplayInfo[]>([]);

  useEffect(() => {
    let alive = true;
    void bluey.capture
      .listDisplays()
      .then((list) => {
        if (alive) setDisplays(list);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  if (!settings) return null;
  const { screen } = settings;

  return (
    <>
      <SectionHeader title="Screen" description="What Bluey can see when you ask about your screen" />

      <SettingRow icon={AppWindowMac} title="Capture target" description="What ⌘↵ captures by default.">
        <Select
          aria-label="Capture target"
          value={screen.captureTarget}
          onChange={(e) => void update({ screen: { captureTarget: e.target.value as typeof screen.captureTarget } })}
          options={[
            { value: "display", label: "Full display" },
            { value: "active_window", label: "Active window" },
            { value: "region", label: "Selected region" },
          ]}
        />
      </SettingRow>

      <SettingRow
        icon={Eye}
        title="Observation"
        description={
          screen.observation === "smart"
            ? "Smart observation samples your screen on an interval so Bluey can prepare answers proactively. It uses more energy and captures more of what you see."
            : "Manual: Bluey only looks at your screen when you ask (⌘↵)."
        }
      >
        <Select
          aria-label="Observation mode"
          value={screen.observation}
          onChange={(e) => void update({ screen: { observation: e.target.value as typeof screen.observation } })}
          options={[
            { value: "manual", label: "Manual" },
            { value: "smart", label: "Smart" },
          ]}
        />
      </SettingRow>

      {screen.observation === "smart" ? (
        <SettingRow icon={Timer} title="Observation interval" description="How often the screen is sampled.">
          <Select
            aria-label="Observation interval"
            value={String(screen.observationIntervalMs)}
            onChange={(e) => void update({ screen: { observationIntervalMs: Number(e.target.value) } })}
            options={[
              { value: "3000", label: "Every 3 seconds" },
              { value: "5000", label: "Every 5 seconds" },
              { value: "10000", label: "Every 10 seconds" },
              { value: "30000", label: "Every 30 seconds" },
            ]}
          />
        </SettingRow>
      ) : null}

      <SettingRow icon={Monitor} title="Preferred display" description="Which display to capture when several are connected.">
        <Select
          aria-label="Preferred display"
          value={screen.preferredDisplay}
          onChange={(e) => void update({ screen: { preferredDisplay: e.target.value } })}
          options={[
            { value: "active", label: "Display with focus" },
            ...displays.map((d) => ({ value: d.id, label: d.name })),
          ]}
        />
      </SettingRow>

      <SectionHeader title="Text recognition" description="On-device OCR for reading your screen" />

      <SettingRow icon={ScanText} title="OCR level" description="Accurate reads more text; fast is lighter.">
        <Select
          aria-label="OCR level"
          value={screen.ocrLevel}
          onChange={(e) => void update({ screen: { ocrLevel: e.target.value as typeof screen.ocrLevel } })}
          options={[
            { value: "fast", label: "Fast" },
            { value: "accurate", label: "Accurate" },
          ]}
        />
      </SettingRow>

      <SettingRow icon={Languages} title="OCR language" description="Primary language of the text on your screen.">
        <Select
          aria-label="OCR language"
          value={screen.ocrLanguages[0] ?? "en-US"}
          onChange={(e) => void update({ screen: { ocrLanguages: [e.target.value] } })}
          options={OCR_LANGUAGES}
        />
      </SettingRow>
    </>
  );
}
