import type { BlueyResponse } from "@/lib/types";

/** Inline markdown that reads as noise once pasted as plain text. Order matters. */
const INLINE_RULES: ReadonlyArray<readonly [RegExp, string]> = [
  [/!\[([^\]]*)\]\([^)]*\)/g, "$1"], // image → alt text
  [/\[([^\]]+)\]\(([^)\s]+)[^)]*\)/g, "$1 ($2)"], // link → text (url)
  [/(\*\*|__)(.+?)\1/g, "$2"], // bold
  [/(^|[^\w*])\*(?!\s)([^*\n]+?)\*(?!\w)/g, "$1$2"], // *italic*
  [/(^|\W)_(?!\s)([^_\n]+?)_(?!\w)/g, "$1$2"], // _italic_
  [/~~(.+?)~~/g, "$1"], // strikethrough
  [/`([^`\n]+)`/g, "$1"], // inline code
];

const FENCE = /^\s*(```|~~~)/;
const HEADING = /^\s{0,3}#{1,6}\s+/;
const QUOTE = /^\s{0,3}>\s?/;

/** Markdown → plain text: emphasis, headings, quotes and fences go; code lines and list items stay. */
export function markdownToPlainText(markdown: string): string {
  let inFence = false;
  const lines: string[] = [];
  for (const line of markdown.split("\n")) {
    if (FENCE.test(line)) {
      inFence = !inFence;
      continue;
    }
    if (inFence) {
      lines.push(line);
      continue;
    }
    const block = line.replace(HEADING, "").replace(QUOTE, "");
    lines.push(INLINE_RULES.reduce((text, [pattern, replacement]) => text.replace(pattern, replacement), block));
  }
  return lines.join("\n").replace(/\n{3,}/g, "\n\n").trim();
}

/** "Copy answer": the title, the body and every section, as plain text (UX-028). */
export function answerPlainText(response: Pick<BlueyResponse, "title" | "content" | "sections">): string {
  const sections = (response.sections ?? []).map((section) => {
    // Code is copied as written: `**kwargs` or `_private` are not emphasis.
    const body = section.kind === "code" ? section.content.trim() : markdownToPlainText(section.content);
    return [section.title.trim(), body].filter(Boolean).join("\n");
  });
  return [response.title?.trim(), markdownToPlainText(response.content), ...sections]
    .filter((part): part is string => Boolean(part))
    .join("\n\n");
}
