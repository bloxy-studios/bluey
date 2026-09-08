import {
  Accessibility,
  ArrowUpToLine,
  Blend,
  Layers,
  LayoutPanelTop,
  MonitorSmartphone,
  MoveHorizontal,
  Palette,
  Rows3,
  Type,
} from "lucide-react";
import { useEffect, useState } from "react";

import { SectionHeader } from "@/components/ui/SectionHeader";
import { Select } from "@/components/ui/Select";
import { SettingRow } from "@/components/ui/SettingRow";
import { Slider } from "@/components/ui/Slider";
import { Switch } from "@/components/ui/Switch";
import { SegmentedTabs } from "@/components/ui/Tabs";
import type { AppearanceSettings } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";

const PANEL_WIDTH_MIN = 520;
const PANEL_WIDTH_MAX = 960;
const PANEL_OPACITY_MIN = 0.4;

/**
 * Settings → Appearance: theme, panel opacity / width / blur, type size,
 * density, always-on-top, position and motion. Theme, font size and motion are
 * mirrored onto `<html data-*>` by the bootstrap; opacity, width and
 * always-on-top are applied natively by the backend's settings side effects.
 */
export default function AppearanceTab() {
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const [opacity, setOpacity] = useState(settings?.appearance.opacity ?? 1);
  const [width, setWidth] = useState(settings?.appearance.width ?? 690);

  useEffect(() => {
    if (!settings) return;
    setOpacity(settings.appearance.opacity);
    setWidth(settings.appearance.width);
  }, [settings]);

  if (!settings) return null;
  const { appearance } = settings;
  const patch = (value: Partial<AppearanceSettings>) => void update({ appearance: value });

  return (
    <>
      <SectionHeader title="Appearance" description="How the HUD and Settings look on your Mac" />

      <SettingRow icon={Palette} title="Theme" description="Follow macOS, or force dark / light.">
        <Select
          aria-label="Theme"
          value={appearance.theme}
          onChange={(e) => patch({ theme: e.target.value as AppearanceSettings["theme"] })}
          options={[
            { value: "system", label: "System" },
            { value: "dark", label: "Dark" },
            { value: "light", label: "Light" },
          ]}
        />
      </SettingRow>

      <SettingRow icon={Type} title="Text size" description="Response and settings type scale.">
        <Select
          aria-label="Text size"
          value={appearance.fontSize}
          onChange={(e) => patch({ fontSize: e.target.value as AppearanceSettings["fontSize"] })}
          options={[
            { value: "small", label: "Small" },
            { value: "medium", label: "Medium" },
            { value: "large", label: "Large" },
          ]}
        />
      </SettingRow>

      <SettingRow icon={Rows3} title="Density" description="Spacing of rows and responses.">
        <SegmentedTabs
          aria-label="Density"
          value={appearance.density}
          onValueChange={(value) => patch({ density: value })}
          options={[
            { value: "compact", label: "Compact" },
            { value: "comfortable", label: "Comfortable" },
          ]}
        />
      </SettingRow>

      <SectionHeader title="HUD panel" description="Size, translucency and placement of the floating panel" />

      <SettingRow
        icon={Blend}
        title="Opacity"
        description={`${Math.round(opacity * 100)}% — the panel keeps its blur.`}
      >
        <Slider
          aria-label="Panel opacity"
          value={opacity}
          min={PANEL_OPACITY_MIN}
          max={1}
          step={0.05}
          onValueChange={setOpacity}
          onValueCommit={(value) => patch({ opacity: Number(value.toFixed(2)) })}
        />
      </SettingRow>

      <SettingRow icon={MoveHorizontal} title="Width" description={`${Math.round(width)} px`}>
        <Slider
          aria-label="Panel width"
          value={width}
          min={PANEL_WIDTH_MIN}
          max={PANEL_WIDTH_MAX}
          step={10}
          onValueChange={setWidth}
          onValueCommit={(value) => patch({ width: Math.round(value) })}
        />
      </SettingRow>

      <SettingRow icon={Layers} title="Background blur" description="Frosted glass behind the panel.">
        <Switch
          aria-label="Background blur"
          checked={appearance.blur}
          onCheckedChange={(blur) => patch({ blur })}
        />
      </SettingRow>

      <SettingRow icon={ArrowUpToLine} title="Always on top" description="Keep the HUD above other windows.">
        <Switch
          aria-label="Always on top"
          checked={appearance.alwaysOnTop}
          onCheckedChange={(alwaysOnTop) => patch({ alwaysOnTop })}
        />
      </SettingRow>

      <SettingRow icon={LayoutPanelTop} title="Position" description="Where the panel appears when shown.">
        <Select
          aria-label="Panel position"
          value={appearance.position}
          onChange={(e) => patch({ position: e.target.value as AppearanceSettings["position"] })}
          options={[
            { value: "remember", label: "Remember last position" },
            { value: "center", label: "Center" },
            { value: "top", label: "Top" },
            { value: "bottom", label: "Bottom" },
            { value: "left", label: "Left" },
            { value: "right", label: "Right" },
          ]}
        />
      </SettingRow>

      <SettingRow
        icon={MonitorSmartphone}
        title="Follow active display"
        description="Move with the display that has focus."
      >
        <Switch
          aria-label="Follow active display"
          checked={appearance.followActiveDisplay}
          onCheckedChange={(followActiveDisplay) => patch({ followActiveDisplay })}
        />
      </SettingRow>

      <SectionHeader title="Motion" description="Animations, pulses and fades" />

      <SettingRow
        icon={Accessibility}
        title="Reduced motion"
        description="Follow the macOS accessibility setting, or override it."
      >
        <Select
          aria-label="Reduced motion"
          value={appearance.reducedMotion}
          onChange={(e) => patch({ reducedMotion: e.target.value as AppearanceSettings["reducedMotion"] })}
          options={[
            { value: "system", label: "System" },
            { value: "on", label: "On" },
            { value: "off", label: "Off" },
          ]}
        />
      </SettingRow>
    </>
  );
}
