import { useEffect, useState } from "react";

import { Select } from "@/components/ui/Select";
import { bluey } from "@/lib/tauri/api";
import type { ResearchBackend } from "@/lib/types";

/**
 * Research backend picker. The installed agent reports which backends it can
 * run (`research_available.agentBackends`): the lite build has no Claude Code
 * CLI, so Claude is disabled there instead of failing at the first ask.
 */
export function ResearchBackendSelect({
  value,
  onChange,
}: {
  value: ResearchBackend;
  onChange(backend: ResearchBackend): void;
}) {
  // `null` until the agent answered — nothing is disabled on a guess.
  const [supported, setSupported] = useState<ResearchBackend[] | null>(null);

  useEffect(() => {
    let alive = true;
    void bluey.research
      .available()
      .then((availability) => {
        if (alive) setSupported(availability.agentBackends);
      })
      .catch(() => {
        // Availability is best-effort; the backend stays selectable.
      });
    return () => {
      alive = false;
    };
  }, []);

  const claudeMissing = supported !== null && !supported.includes("claude");
  return (
    <Select
      aria-label="Research backend"
      value={value}
      onChange={(e) => onChange(e.target.value as ResearchBackend)}
      options={[
        { value: "gemini", label: "Gemini" },
        {
          value: "claude",
          label: claudeMissing ? "Claude (full build only)" : "Claude",
          // A saved Claude choice stays visible so the user can switch away.
          disabled: claudeMissing && value !== "claude",
        },
      ]}
    />
  );
}
