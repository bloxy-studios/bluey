import { cn } from "@/lib/utils/cn";
import { acceleratorToGlyphs } from "@/lib/utils/keyboard";

export interface KeycapProps {
  glyph: string;
  className?: string;
  /** Accent highlight while recording a shortcut. */
  recording?: boolean;
}

/** A single 24×24 keycap (wider for glyph pairs), per DESIGN.md Keybinds. */
export function Keycap({ glyph, className, recording }: KeycapProps) {
  return (
    <kbd
      className={cn(
        "inline-flex h-6 min-w-6 items-center justify-center rounded-keycap bg-bg-tile px-1.5",
        "font-sans text-[12px] leading-none text-fg-muted",
        recording && "bg-accent-soft text-accent",
        className,
      )}
    >
      {glyph}
    </kbd>
  );
}

export interface KeycapsProps {
  accelerator: string;
  className?: string;
  recording?: boolean;
}

/** Renders a Tauri accelerator ("CmdOrCtrl+Shift+Enter") as keycaps ⌘ ⇧ ↵. */
export function Keycaps({ accelerator, className, recording }: KeycapsProps) {
  const glyphs = acceleratorToGlyphs(accelerator);
  return (
    <span className={cn("inline-flex items-center gap-1", className)} aria-label={glyphs.join(" ")}>
      {glyphs.map((glyph, index) => (
        <Keycap key={`${glyph}-${index}`} glyph={glyph} recording={recording} />
      ))}
    </span>
  );
}
