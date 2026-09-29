import { TriangleAlert } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/Button";
import type { PermissionKind } from "@/lib/types";

const LABELS: Record<PermissionKind, string> = {
  screenRecording: "Screen Recording",
  microphone: "Microphone",
  accessibility: "Accessibility",
  speechRecognition: "Speech Recognition",
  notifications: "Notifications",
};

interface Props {
  lost: readonly PermissionKind[];
  onOpenSystemSettings: (kind: PermissionKind) => void;
}

/**
 * MAC-001: an update signed with another code identity (every build without a
 * Developer ID) no longer matches the grants macOS gave the previous version.
 * One card names what was lost and links each pane.
 */
export function UpdateRepairCard({ lost, onOpenSystemSettings }: Props) {
  const [dismissed, setDismissed] = useState(false);
  if (lost.length === 0 || dismissed) return null;
  return (
    <section
      aria-labelledby="update-repair-title"
      className="mt-4 rounded-card border border-danger/30 bg-danger/8 p-4"
    >
      <div className="flex items-start gap-3">
        <TriangleAlert className="mt-0.5 size-5 shrink-0 text-danger" strokeWidth={1.8} aria-hidden />
        <div className="min-w-0 flex-1">
          <h3 id="update-repair-title" className="text-[14px] font-semibold text-fg">
            macOS turned off {lost.length === 1 ? "a permission" : "some permissions"} after the update
          </h3>
          <p className="mt-1 text-[12.5px] leading-relaxed text-fg-muted">
            This build of Bluey is not signed with an Apple Developer ID, so macOS treats each update as a new app:
            permissions and Keychain approvals given to the previous version no longer apply. Turn these back on in
            System Settings. If Bluey is already listed there, remove it with “–” and add it again. macOS may also
            ask once more before Bluey can read your saved API keys.
          </p>
          <ul className="mt-3 flex flex-col gap-1.5">
            {lost.map((kind) => (
              <li key={kind} className="flex items-center justify-between gap-3">
                <span className="text-[13px] font-medium text-fg">{LABELS[kind]}</span>
                <Button
                  size="sm"
                  variant="secondary"
                  aria-label={`Open System Settings for ${LABELS[kind]}`}
                  onClick={() => onOpenSystemSettings(kind)}
                >
                  Open System Settings
                </Button>
              </li>
            ))}
          </ul>
        </div>
        <Button size="sm" variant="ghost" onClick={() => setDismissed(true)}>
          Dismiss
        </Button>
      </div>
    </section>
  );
}
