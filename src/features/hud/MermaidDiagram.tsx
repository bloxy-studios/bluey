import { useEffect, useRef, useState } from "react";

import { Spinner } from "@/components/ui/Spinner";
import { createId } from "@/lib/utils/id";

export interface MermaidDiagramProps {
  source: string;
}

let initialized = false;

/** Lazily renders a mermaid diagram (system-design responses). */
export function MermaidDiagram({ source }: MermaidDiagramProps) {
  const [svg, setSvg] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const idRef = useRef(createId("mermaid"));

  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const mermaid = (await import("mermaid")).default;
        if (!initialized) {
          mermaid.initialize({
            startOnLoad: false,
            theme: "dark",
            darkMode: true,
            fontFamily: "-apple-system, 'SF Pro Text', Inter, system-ui, sans-serif",
          });
          initialized = true;
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
  }, [source]);

  if (failed) {
    return (
      <pre className="my-3 overflow-x-auto rounded-[10px] border border-hud-border bg-[#0d0d0d] p-3.5 font-mono text-[12.5px] text-fg-muted">
        {source}
      </pre>
    );
  }
  if (!svg) {
    return (
      <div className="my-3 flex h-24 items-center justify-center rounded-[10px] border border-hud-border bg-[#0d0d0d]">
        <Spinner />
      </div>
    );
  }
  return (
    <div
      className="my-3 overflow-x-auto rounded-[10px] border border-hud-border bg-[#0d0d0d] p-3 [&_svg]:mx-auto"
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  );
}
