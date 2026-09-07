import { Pill } from "@/components/ui/Pill";
import { Spinner } from "@/components/ui/Spinner";
import { useAppStore } from "@/stores/appStore";
import { useChatStore } from "@/stores/chatStore";
import { modeById, useModesStore } from "@/stores/modesStore";
import { derivePill } from "./state-pill";

/** The HUD's left status pill: `Bluey · General` / `● Listening` / `◌ Thinking`… */
export function StatePill() {
  const status = useAppStore((s) => s.status);
  const phase = useChatStore((s) => s.phase);
  const prepared = useChatStore((s) => s.prepared);
  const modes = useModesStore((s) => s.modes);
  const modeName = modeById(modes, status?.modeId)?.name ?? "General";

  const pill = derivePill(status, phase, prepared !== null, modeName);

  switch (pill.kind) {
    case "error":
      return (
        <Pill variant="hud" className="text-danger">
          <span aria-hidden>!</span> Something went wrong
        </Pill>
      );
    case "reading":
      return (
        <Pill variant="hud">
          <Spinner size={11} /> Reading screen
        </Pill>
      );
    case "thinking":
      return (
        <Pill variant="hud">
          <Spinner size={11} /> Thinking
        </Pill>
      );
    case "prepared":
      return (
        <Pill variant="accent" className="motion-safe:animate-fade-in">
          Bluey has a suggestion · ⌘⇧↵
        </Pill>
      );
    case "listening":
      return (
        <Pill variant="hud">
          <span className="size-[7px] rounded-full bg-success motion-safe:animate-pulse-dot" aria-hidden />
          Listening
        </Pill>
      );
    case "idle":
      return <Pill variant="hud">Bluey · {pill.modeName}</Pill>;
  }
}
