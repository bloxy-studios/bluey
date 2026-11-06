/**
 * Streaming-markdown helpers. While a fenced code block is still open the
 * draft is rendered with a temporary closing fence, so code streams in line by
 * line (a coding answer opens with its solution — hiding the open block left
 * the HUD blank until the whole solution was written). The final answer is
 * rendered as-is.
 */

export interface StreamingMarkdown {
  /** Markdown safe to render right now (an open block closed temporarily). */
  renderable: string;
  /** True when a fenced code block is still open (still being written). */
  pendingCode: boolean;
  /** The open block's code so far, so its Copy can wait until it is complete. */
  openCode?: string;
}

const FENCE = /^(`{3,}|~{3,})/;

export function splitStreamingMarkdown(content: string): StreamingMarkdown {
  const lines = content.split("\n");
  let openFence: string | null = null;
  let openLine = -1;
  for (const [index, line] of lines.entries()) {
    const trimmed = line.trim();
    const marker = FENCE.exec(trimmed)?.[1];
    if (!marker) continue;
    if (openFence === null) {
      openFence = marker;
      openLine = index;
    } else if (trimmed === marker && marker[0] === openFence[0] && marker.length >= openFence.length) {
      // Only a bare fence at least as long as the opener closes it (CommonMark).
      openFence = null;
    }
  }
  if (openFence === null) return { renderable: content, pendingCode: false };
  const body = lines.slice(openLine + 1).join("\n");
  const separator = content.endsWith("\n") ? "" : "\n";
  return {
    renderable: `${content}${separator}${openFence}`,
    pendingCode: true,
    openCode: body.replace(/\n$/, ""),
  };
}

/** Extract all fenced code blocks (for "Copy code"). */
export function extractCodeBlocks(content: string): Array<{ language: string; code: string }> {
  const blocks: Array<{ language: string; code: string }> = [];
  const pattern = /```([\w+#-]*)\n([\s\S]*?)```/g;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(content)) !== null) {
    blocks.push({ language: match[1] ?? "", code: (match[2] ?? "").replace(/\n$/, "") });
  }
  return blocks;
}
