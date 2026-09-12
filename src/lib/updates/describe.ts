/**
 * Copy for the in-app updater's status (docs/UPDATES.md) — one place for the
 * HUD pill label and the Settings → General → Updates row, so both say the
 * same thing about the same state. Pure.
 */

import type { UpdateStatus } from "@/lib/types";

export type UpdateAction = "check" | "install" | "relaunch";

export interface UpdatePrimaryAction {
  label: string;
  action: UpdateAction | null;
  /** Nothing to do right now (a check or download is in flight, or the build cannot update). */
  disabled: boolean;
}

export function downloadPercent(status: UpdateStatus): number | undefined {
  const progress = status.progress;
  if (!progress?.total) return undefined;
  return Math.min(100, Math.round((progress.downloaded / progress.total) * 100));
}

/** The HUD pill's text for the phases that earn a pill (`available` / `downloading` / `ready`). */
export function updatePillLabel(status: UpdateStatus): string | null {
  switch (status.phase) {
    case "available":
      return status.available ? `Update available · ${status.available.version}` : null;
    case "downloading": {
      const percent = downloadPercent(status);
      return percent === undefined ? "Updating…" : `Updating… ${percent}%`;
    }
    case "ready":
      return "Restart to update";
    default:
      return null;
  }
}

function relativeTime(iso: string, now: Date): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "";
  const minutes = Math.max(0, Math.round((now.getTime() - then) / 60_000));
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.round(hours / 24);
  return `${days} d ago`;
}

/** One sentence for the Settings row under "Bluey <version>". */
export function describeUpdateStatus(status: UpdateStatus | null, now: () => Date = () => new Date()): string {
  if (!status) return "Checking the updater…";
  if (!status.supported) return "Development builds don't update themselves — install a release from the download page.";
  const checked = status.lastCheckedAt ? ` · checked ${relativeTime(status.lastCheckedAt, now())}` : "";
  switch (status.phase) {
    case "idle":
      return "Checks run 30 seconds after launch and every 6 hours.";
    case "checking":
      return "Checking for updates…";
    case "up_to_date":
      return `Up to date${checked}.`;
    case "available":
      return status.available
        ? `Version ${status.available.version} is available${status.automatic ? " — downloading shortly" : ""}.`
        : "An update is available.";
    case "downloading": {
      const percent = downloadPercent(status);
      const version = status.available?.version ?? "the update";
      return percent === undefined ? `Downloading ${version}…` : `Downloading ${version} · ${percent}%`;
    }
    case "ready":
      return `${status.available?.version ?? "The update"} is installed — restart Bluey to finish.`;
    case "error":
      return `Couldn't check for updates${checked}. Bluey keeps running this version.`;
  }
}

/** The one button the Settings row shows for the current phase. */
export function primaryUpdateAction(status: UpdateStatus | null, busy = false): UpdatePrimaryAction {
  if (!status || !status.supported) return { label: "Check now", action: "check", disabled: !status || busy };
  switch (status.phase) {
    case "checking":
      return { label: "Checking…", action: null, disabled: true };
    case "available":
      return { label: "Install", action: "install", disabled: busy };
    case "downloading":
      return { label: "Updating…", action: null, disabled: true };
    case "ready":
      return { label: "Restart to update", action: "relaunch", disabled: false };
    default:
      return { label: "Check now", action: "check", disabled: busy };
  }
}
