import { useEffect, useRef, useState } from "react";

import { Spinner } from "@/components/ui/Spinner";
import { createId } from "@/lib/utils/id";
import { useDocumentTheme, type DocumentTheme } from "./useDocumentTheme";

export interface MermaidDiagramProps {
  source: string;
}

let initializedTheme: DocumentTheme | null = null;

/** Lazily renders a mermaid diagram (system-design responses). */
export function MermaidDiagram({ source }: MermaidDiagramProps) {
  const [svg, setSvg] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const idRef = useRef(createId("mermaid"));
  const theme = useDocumentTheme();

  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const mermaid = (await import("mermaid")).default;
        if (initializedTheme !== theme) {
          mermaid.initialize({
            startOnLoad: false,
            theme: theme === "light" ? "default" : "dark",
            darkMode: theme === "dark",
            fontFamily: "-apple-system, 'SF Pro Text', Inter, system-ui, sans-serif",
          });
          initializedTheme = theme;
        }
        const { svg: rendered } = await mermaid.render(idRef.current, source);
        if (alive) setSvg(rendered);
      } catch (error) {
        console.warn("[mermaid] render failed", error);
        if (alive) setFailed(true);
      }
    })();
    return () => {
      alive = false;
    };
  }, [source, theme]);

  if (failed) {
    return (
      <pre className="my-3 overflow-x-auto rounded-[10px] border border-hud-border bg-code-bg p-3.5 font-mono text-[12.5px] text-fg-muted">
        {source}
      </pre>
    );
  }
  if (!svg) {
    return (
      <div className="my-3 flex h-24 items-center justify-center rounded-[10px] border border-hud-border bg-code-bg">
        <Spinner />
      </div>
    );
  }
  return (
    <div
      className="my-3 overflow-x-auto rounded-[10px] border border-hud-border bg-code-bg p-3 [&_svg]:mx-auto"
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  );
}
