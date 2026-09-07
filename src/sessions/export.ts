/** Markdown export composer for session previews. */

import type { BlueyMode, BlueyResponse, Session, SessionNote, SessionSummary } from "@/lib/types";
import type { TimelineGroup } from "./timeline";

export interface ComposeSessionMarkdownArgs {
  session: Session;
  mode: BlueyMode;
  summary?: SessionSummary | null;
  timeline?: TimelineGroup[];
  responses?: BlueyResponse[];
  notes?: SessionNote[];
}

function list(items: string[]): string {
  return items.map((item) => `- ${item}`).join("\n");
}

function section(title: string, body: string): string {
  return body.trim().length > 0 ? `## ${title}\n\n${body}` : "";
}

export function composeSessionMarkdown(args: ComposeSessionMarkdownArgs): string {
  const { session, mode, summary, timeline, responses, notes } = args;
  const title = session.title ?? `${mode.name} session`;
  const started = new Date(session.startedAt);
  const dateLine = Number.isNaN(started.getTime())
    ? session.startedAt
    : started.toLocaleString();

  const parts: string[] = [`# ${title}`, `*${mode.name} — ${dateLine}*`];

  if (summary) {
    parts.push(section("Overview", summary.overview));
    if (summary.topics.length > 0) parts.push(section("Topics", list(summary.topics)));
    if (summary.questions.length > 0) parts.push(section("Questions", list(summary.questions)));
    if (summary.answers.length > 0) parts.push(section("Key answers", list(summary.answers)));
    if (summary.decisions.length > 0) parts.push(section("Decisions", list(summary.decisions)));
    if (summary.actionItems.length > 0) parts.push(section("Action items", list(summary.actionItems)));
    if (summary.openItems.length > 0) parts.push(section("Open items", list(summary.openItems)));
    if (summary.improvements.length > 0) parts.push(section("Improvements", list(summary.improvements)));
    for (const extra of summary.sections ?? []) {
      parts.push(section(extra.title, extra.content));
    }
  }

  if (timeline && timeline.length > 0) {
    const lines = timeline.map((group) => `- **${group.time}** — ${group.entries.join(" / ")}`);
    parts.push(section("Timeline", lines.join("\n")));
  }

  if (responses && responses.length > 0) {
    const blocks = responses.map((response) => {
      const heading = response.title ?? response.type;
      return `### ${heading}\n\n${response.content}`;
    });
    parts.push(section("Responses", blocks.join("\n\n")));
  }

  if (notes && notes.length > 0) {
    parts.push(section("Notes", list(notes.map((note) => note.content))));
  }

  return parts.filter((part) => part.length > 0).join("\n\n").trim() + "\n";
}
