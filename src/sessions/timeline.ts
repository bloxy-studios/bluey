/**
 * Session timeline: groups events + responses by minute, e.g.
 * "09:14 — Question detected / Response prepared".
 */

import type { BlueyResponse, SessionEvent } from "@/lib/types";

export interface TimelineGroup {
  /** "HH:MM" local time. */
  time: string;
  /** ISO timestamp of the first entry in the group. */
  at: string;
  entries: string[];
}

function minuteKey(iso: string): { key: string; label: string } {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return { key: iso, label: iso };
  const hh = String(date.getHours()).padStart(2, "0");
  const mm = String(date.getMinutes()).padStart(2, "0");
  const label = `${hh}:${mm}`;
  // Key includes the day so multi-day sessions do not merge.
  const key = `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}T${label}`;
  return { key, label };
}

function responseLabel(response: BlueyResponse): string {
  const verb = response.prepared ? "Response prepared" : "Response generated";
  return response.title ? `${verb}: ${response.title}` : verb;
}

export function buildTimeline(events: SessionEvent[], responses: BlueyResponse[]): TimelineGroup[] {
  interface Entry {
    at: string;
    label: string;
  }
  const entries: Entry[] = [
    ...events.map((event) => ({
      at: event.createdAt,
      label: event.detail ? `${event.title}: ${event.detail}` : event.title,
    })),
    ...responses.map((response) => ({ at: response.createdAt, label: responseLabel(response) })),
  ].sort((a, b) => new Date(a.at).getTime() - new Date(b.at).getTime());

  const groups: TimelineGroup[] = [];
  const byKey = new Map<string, TimelineGroup>();
  for (const entry of entries) {
    const { key, label } = minuteKey(entry.at);
    let group = byKey.get(key);
    if (!group) {
      group = { time: label, at: entry.at, entries: [] };
      byKey.set(key, group);
      groups.push(group);
    }
    group.entries.push(entry.label);
  }
  return groups;
}
