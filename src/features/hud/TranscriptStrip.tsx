import { ChevronDown, ChevronUp } from "lucide-react";

import { IconButton } from "@/components/ui/IconButton";
import { Pill } from "@/components/ui/Pill";
import { cn } from "@/lib/utils/cn";
import { useAppStore } from "@/stores/appStore";
import { useHudUiStore } from "@/stores/hudUiStore";
import { modeById, useModesStore } from "@/stores/modesStore";
import { useTranscriptStore } from "@/stores/transcriptStore";
import { buildTranscriptLines, COLLAPSED_LINES, EXPANDED_LINES } from "./transcript-strip";

/**
 * Live transcript strip: the last few finalized segments plus the in-flight
 * partial while an audio session is running. Speaker labels are heuristic, so
 * a low-confidence label is dimmed. Collapsible to a single line.
 */
export function TranscriptStrip() {
  const status = useAppStore((s) => s.status);
  const segments = useTranscriptStore((s) => s.segments);
  const partial = useTranscriptStore((s) => s.partial);
  const questions = useTranscriptStore((s) => s.questions);
  const modes = useModesStore((s) => s.modes);
  const collapsed = useHudUiStore((s) => s.transcriptCollapsed);
  const toggle = useHudUiStore((s) => s.toggleTranscript);

  if (!status?.audioActive) return null;

  const mode = modeById(modes, status.modeId);
  const lines = buildTranscriptLines(
    segments,
    partial,
    questions,
    mode,
    collapsed ? COLLAPSED_LINES : EXPANDED_LINES,
  );

  return (
    <section aria-label="Live transcript" className="shrink-0 border-t border-hud-border px-4 pb-2 pt-1.5">
      <div className="flex h-6 items-center gap-2">
        <span className="size-[6px] rounded-full bg-success motion-safe:animate-pulse-dot" aria-hidden />
        <span className="text-[11px] font-medium uppercase tracking-wide text-fg-subtle">
          Live transcript
        </span>
        <span className="flex-1" />
        <IconButton
          aria-label={collapsed ? "Expand transcript" : "Collapse transcript"}
          aria-expanded={!collapsed}
          size="sm"
          onClick={toggle}
        >
          {collapsed ? (
            <ChevronUp className="size-3.5" aria-hidden />
          ) : (
            <ChevronDown className="size-3.5" aria-hidden />
          )}
        </IconButton>
      </div>

      {lines.length === 0 ? (
        <p className="m-0 text-[12.5px] italic text-fg-subtle">Waiting for speech…</p>
      ) : (
        <ul className="m-0 flex list-none flex-col gap-0.5 p-0">
          {lines.map((line) => (
            <li key={line.id} className="flex items-baseline gap-2 text-[12.5px] leading-snug">
              <span
                className={cn(
                  "shrink-0 font-medium",
                  line.speakerConfidence >= 0.7 ? "text-fg-muted" : "text-fg-subtle",
                )}
                title={`Speaker confidence ${Math.round(line.speakerConfidence * 100)}%`}
              >
                {line.speaker}
              </span>
              <span
                className={cn("min-w-0 flex-1 truncate", line.partial ? "italic text-fg-subtle" : "text-fg")}
              >
                {line.text}
              </span>
              {line.detected ? (
                <Pill size="sm" variant="muted" className="shrink-0">
                  Question
                </Pill>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
