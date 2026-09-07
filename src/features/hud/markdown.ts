/**
 * Streaming-markdown helpers. Per DESIGN.md, code blocks are buffered until
 * their closing fence: while a fence is open we cut the partial block from the
 * rendered markdown and show a "writing code" placeholder instead.
 */

export interface StreamingMarkdown {
  /** Markdown safe to render right now. */
  renderable: string;
  /** True when a fenced code block is still open (buffered). */
  pendingCode: boolean;
}

const FENCE = /^(```|~~~)/;

export function splitStreamingMarkdown(content: string): StreamingMarkdown {
  const lines = content.split("\n");
  let open = false;
  let lastFenceLine = -1;
  lines.forEach((line, index) => {
    if (FENCE.test(line.trimStart())) {
      open = !open;
      if (open) lastFenceLine = index;
    }
  });
  if (!open || lastFenceLine < 0) return { renderable: content, pendingCode: false };
  return { renderable: lines.slice(0, lastFenceLine).join("\n"), pendingCode: true };
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
