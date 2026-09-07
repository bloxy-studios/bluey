import { Check, ChevronsDownUp, ChevronsUpDown, Copy } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { cn } from "@/lib/utils/cn";
import { copyText } from "@/lib/utils/clipboard";
import { highlightCode } from "./highlighter";

const COLLAPSE_LINE_COUNT = 16;

export interface CodeBlockProps {
  code: string;
  language?: string;
  className?: string;
}

/**
 * Fenced code block: header bar with language + Copy ("Copied" for 1s) +
 * Expand/Collapse for long code; shiki highlighting loaded lazily with a
 * plain fallback while it loads.
 */
export function CodeBlock({ code, language, className }: CodeBlockProps) {
  const [html, setHtml] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const lineCount = code.split("\n").length;
  const collapsible = lineCount > COLLAPSE_LINE_COUNT;
  const [expanded, setExpanded] = useState(!collapsible);
  const copyTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    let alive = true;
    const theme = document.documentElement.dataset.theme === "light" ? "light" : "dark";
    void highlightCode(code, language, theme).then((result) => {
      if (alive) setHtml(result);
    });
    return () => {
      alive = false;
    };
  }, [code, language]);

  useEffect(() => () => {
    if (copyTimer.current) clearTimeout(copyTimer.current);
  }, []);

  const onCopy = async () => {
    if (await copyText(code)) {
      setCopied(true);
      if (copyTimer.current) clearTimeout(copyTimer.current);
      copyTimer.current = setTimeout(() => setCopied(false), 1000);
    }
  };

  return (
    <div className={cn("code-block group my-3 overflow-hidden rounded-[10px] border border-hud-border bg-[#0d0d0d]", className)}>
      <div className="flex h-8 items-center justify-between border-b border-hud-border bg-white/4 pl-3 pr-1.5">
        <span className="font-mono text-[11px] uppercase tracking-wide text-fg-subtle">{language || "code"}</span>
        <div className="flex items-center gap-0.5">
          {collapsible ? (
            <button
              type="button"
              onClick={() => setExpanded((v) => !v)}
              className="flex h-6 items-center gap-1 rounded-[6px] px-2 text-[11.5px] text-fg-muted transition-colors hover:bg-white/8 hover:text-fg"
            >
              {expanded ? <ChevronsDownUp className="size-3.5" aria-hidden /> : <ChevronsUpDown className="size-3.5" aria-hidden />}
              {expanded ? "Collapse" : "Expand"}
            </button>
          ) : null}
          <button
            type="button"
            onClick={() => void onCopy()}
            aria-label="Copy code"
            className="flex h-6 items-center gap-1 rounded-[6px] px-2 text-[11.5px] text-fg-muted transition-colors hover:bg-white/8 hover:text-fg"
          >
            {copied ? <Check className="size-3.5 text-success" aria-hidden /> : <Copy className="size-3.5" aria-hidden />}
            {copied ? "Copied" : "Copy"}
          </button>
        </div>
      </div>
      <div className={cn("selectable relative", !expanded && "max-h-[300px] overflow-hidden")}>
        {html ? (
          // shiki output is trusted local rendering of the code string
          <div dangerouslySetInnerHTML={{ __html: html }} />
        ) : (
          <pre className="m-0 overflow-x-auto p-3.5 font-mono text-[13px] leading-relaxed text-fg">
            <code>{code}</code>
          </pre>
        )}
        {!expanded ? (
          <div className="pointer-events-none absolute inset-x-0 bottom-0 h-16 bg-gradient-to-t from-[#0d0d0d] to-transparent" />
        ) : null}
      </div>
      {!expanded ? (
        <button
          type="button"
          onClick={() => setExpanded(true)}
          className="block w-full border-t border-hud-border py-1.5 text-center text-[12px] font-medium text-fg-muted transition-colors hover:bg-white/4 hover:text-fg"
        >
          Expand solution ({lineCount} lines)
        </button>
      ) : null}
    </div>
  );
}
