import { AudioLines, Globe, Mic, SlidersHorizontal, UsersRound, Waves } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/Button";
import { LevelMeter } from "@/components/ui/LevelMeter";
import { SectionHeader } from "@/components/ui/SectionHeader";
import { Select } from "@/components/ui/Select";
import { SettingRow } from "@/components/ui/SettingRow";
import { Switch } from "@/components/ui/Switch";
import { bluey } from "@/lib/tauri/api";
import type { AudioDevice } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";
import { useTranscriptStore } from "@/stores/transcriptStore";

const LANGUAGE_OPTIONS = [
  { value: "auto", label: "Auto-detect" },
  { value: "en", label: "English" },
  { value: "es", label: "Spanish" },
  { value: "fr", label: "French" },
  { value: "de", label: "German" },
  { value: "pt", label: "Portuguese" },
  { value: "ja", label: "Japanese" },
  { value: "zh", label: "Chinese" },
];

export default function AudioTab() {
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const levels = useTranscriptStore((s) => s.levels);
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [testing, setTesting] = useState(false);
  const [peak, setPeak] = useState<number | null>(null);

  useEffect(() => {
    let alive = true;
    void bluey.audio
      .listDevices()
      .then((list) => {
        if (alive) setDevices(list.filter((d) => d.kind === "input"));
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  if (!settings) return null;
  const { audio } = settings;

  const testMicrophone = async () => {
    setTesting(true);
    setPeak(null);
    try {
      const result = await bluey.audio.testMicrophone({
        deviceId: audio.microphoneDeviceId,
        durationMs: 2000,
      });
      setPeak(result.peakLevel);
    } catch (error) {
      console.warn("[audio] test failed", error);
    } finally {
      setTesting(false);
    }
  };

  return (
    <>
      <SectionHeader title="Audio" description="Choose how Bluey listens during sessions" />

      <SettingRow
        icon={AudioLines}
        title="Audio source"
        description="What Bluey transcribes during a session."
      >
        <Select
          aria-label="Audio source"
          value={audio.source}
          onChange={(e) => void update({ audio: { source: e.target.value as typeof audio.source } })}
          options={[
            { value: "microphone", label: "Microphone only" },
            { value: "system", label: "System audio only" },
            { value: "both", label: "Microphone + system audio" },
          ]}
        />
      </SettingRow>

      <SettingRow icon={Mic} title="Microphone" description="The input device used for your voice.">
        <Select
          aria-label="Microphone device"
          value={audio.microphoneDeviceId ?? devices.find((d) => d.isDefault)?.id ?? ""}
          onChange={(e) => void update({ audio: { microphoneDeviceId: e.target.value } })}
          options={devices.map((d) => ({ value: d.id, label: d.isDefault ? `Default — ${d.name}` : d.name }))}
        />
      </SettingRow>

      <SettingRow
        icon={Globe}
        title="Transcription language"
        description="Select the language you speak in meetings."
      >
        <Select
          aria-label="Transcription language"
          value={audio.transcriptionLanguage}
          onChange={(e) => void update({ audio: { transcriptionLanguage: e.target.value } })}
          options={LANGUAGE_OPTIONS}
        />
      </SettingRow>

      <SettingRow
        icon={UsersRound}
        title="Speaker identification"
        description="Label who is speaking (never fully certain)."
      >
        <Switch
          aria-label="Speaker identification"
          checked={audio.speakerIdentification}
          onCheckedChange={(checked) => void update({ audio: { speakerIdentification: checked } })}
        />
      </SettingRow>

      <SettingRow
        icon={Waves}
        title="Transcription provider"
        description="On-device Apple Speech, or cloud (MAI-Transcribe-1.5 over Voice Live)."
      >
        <Select
          aria-label="Transcription provider"
          value={audio.transcriptionProvider}
          onChange={(e) =>
            void update({
              audio: { transcriptionProvider: e.target.value as typeof audio.transcriptionProvider },
            })
          }
          options={[
            { value: "apple", label: "Apple (on-device)" },
            { value: "cloud_realtime", label: "Cloud realtime" },
          ]}
        />
      </SettingRow>

      <SettingRow
        icon={SlidersHorizontal}
        title="Voice activity sensitivity"
        description="Higher picks up quieter speech; lower ignores noise."
      >
        <Select
          aria-label="VAD sensitivity"
          value={audio.vadSensitivity}
          onChange={(e) =>
            void update({ audio: { vadSensitivity: e.target.value as typeof audio.vadSensitivity } })
          }
          options={[
            { value: "low", label: "Low" },
            { value: "medium", label: "Medium" },
            { value: "high", label: "High" },
          ]}
        />
      </SettingRow>

      <SectionHeader title="Audio check" description="Test your audio input before you hop into a call" />

      <SettingRow
        icon={Mic}
        title="Microphone test"
        description={peak !== null ? `Peak level ${(peak * 100).toFixed(0)}%` : "Speak while the meter runs."}
      >
        {testing ? <LevelMeter level={levels.microphone} aria-label="Microphone level" /> : null}
        <Button variant="secondary" onClick={() => void testMicrophone()} disabled={testing}>
          {testing ? "Listening…" : "Test Microphone"}
        </Button>
      </SettingRow>
    </>
  );
}
