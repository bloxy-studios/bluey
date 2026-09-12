import { ChevronRight, ExternalLink } from "lucide-react";
import { lazy, Suspense, useState, type ComponentProps } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { ErrorBanner } from "@/components/ui/ErrorBanner";
import { Spinner } from "@/components/ui/Spinner";
import { unreadableAnswerError } from "@/lib/errors/answers";
import type { BlueyResponse, ResponseSection, ResponseType } from "@/lib/types";
import { cn } from "@/lib/utils/cn";
import { openExternal } from "@/lib/utils/open-external";
import { looksLikeStructuredJson } from "@/lib/utils/partial-json";
import { CodeBlock } from "./CodeBlock";
import { splitStreamingMarkdown } from "./markdown";

const MermaidDiagram = lazy(() => import("./MermaidDiagram").then((m) => ({ default: m.MermaidDiagram })));

type CodeProps = ComponentProps<"code"> & { node?: unknown };

function MarkdownCode({ className, children, ...props }: CodeProps) {
  const match = /language-([\w+#-]+)/.exec(className ?? "");
  const text = String(children ?? "").replace(/\n$/, "");
  if (match || text.includes("\n")) {
    return <CodeBlock code={text} language={match?.[1]} />;
  }
  return (
    <code className={className} {...props}>
      {children}
    </code>
  );
}

const MARKDOWN_COMPONENTS = {
  code: MarkdownCode,
  pre: ({ children }: ComponentProps<"pre">) => <>{children}</>,
  a: ({ href, children }: ComponentProps<"a">) => (
    <a
      href={href}
      onClick={(event) => {
        event.preventDefault();
        if (href) void openExternal(href);
      }}
    >
      {children}
    </a>
  ),
};

function Markdown({ content }: { content: string }) {
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={MARKDOWN_COMPONENTS}>
      {content}
    </ReactMarkdown>
  );
}

/**
 * Response types whose sections are part of the answer the user reads or
 * says — rendered as plain headed blocks, not collapsed away. Code, design and
 * artefact-heavy kinds keep the collapsible chrome.
 */
const PLAIN_SECTION_TYPES: ReadonlySet<ResponseType> = new Set<ResponseType>(["answer", "suggestion", "research", "summary"]);
const COLLAPSIBLE_KINDS: ReadonlySet<NonNullable<ResponseSection["kind"]>> = new Set(["code", "diagram", "table", "calculation"]);

function isCollapsible(responseType: ResponseType, section: ResponseSection): boolean {
  if (!PLAIN_SECTION_TYPES.has(responseType)) return true;
  return section.kind !== undefined && COLLAPSIBLE_KINDS.has(section.kind);
}

function HeadedSection({ section }: { section: ResponseSection }) {
  return (
    <section className="mt-3" data-testid="headed-section">
      <h3 className="mb-1 mt-0 text-[11px] font-medium uppercase tracking-wide text-fg-subtle">{section.title}</h3>
      <Markdown content={section.content} />
    </section>
  );
}

function CollapsibleSection({ section }: { section: ResponseSection }) {
  const [open, setOpen] = useState(!section.collapsed);
  return (
    <section className="mt-3 overflow-hidden rounded-[10px] border border-hud-border">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="flex w-full items-center gap-2 bg-white/4 px-3 py-2 text-left text-[13px] font-semibold text-fg transition-colors hover:bg-white/8"
      >
        <ChevronRight className={cn("size-3.5 text-fg-muted transition-transform", open && "rotate-90")} aria-hidden />
        {section.title}
      </button>
      {open ? (
        <div className="px-3.5 py-3">
          {section.kind === "code" ? (
            <CodeBlock code={section.content} language={section.language} />
          ) : section.kind === "diagram" ? (
            <Suspense fallback={<Spinner />}>
              <MermaidDiagram source={section.content} />
            </Suspense>
          ) : (
            <Markdown content={section.content} />
          )}
        </div>
      ) : null}
    </section>
  );
}

export interface ResponseViewProps {
  response: BlueyResponse;
  /** True while this response is still streaming in. */
  streaming?: boolean;
  className?: string;
}

/**
 * Renders a BlueyResponse: markdown body, sections, code, diagram, citations.
 * Content that is (or was meant to be) a JSON envelope is never rendered —
 * the engine's parser should have unwrapped or salvaged it; if raw JSON still
 * arrives here, the user sees the "couldn't read the answer" state instead.
 */
export function ResponseView({ response, streaming, className }: ResponseViewProps) {
  const { renderable, pendingCode } = streaming
    ? splitStreamingMarkdown(response.content)
    : { renderable: response.content, pendingCode: false };
  const jsonLike = looksLikeStructuredJson(renderable);
  const unreadable = jsonLike && !streaming;

  return (
    <div className={cn("response-body selectable motion-safe:animate-fade-in", className)}>
      {response.title && !unreadable ? (
        <h2 className="mb-2 mt-0 text-[16px] font-semibold leading-snug">{response.title}</h2>
      ) : null}

      {unreadable ? (
        <ErrorBanner error={unreadableAnswerError()} compact />
      ) : jsonLike ? null : (
        <Markdown content={renderable} />
      )}

      {pendingCode ? (
        <div className="my-3 flex items-center gap-2 rounded-[10px] border border-hud-border bg-[#0d0d0d] px-3.5 py-3 text-[13px] text-fg-muted">
          <Spinner size={12} />
          Writing code…
        </div>
      ) : null}

      {response.code && !response.content.includes(response.code.code) ? (
        <CodeBlock code={response.code.code} language={response.code.language} />
      ) : null}

      {response.diagram ? (
        <Suspense
          fallback={
            <div className="my-3 flex h-24 items-center justify-center rounded-[10px] border border-hud-border bg-[#0d0d0d]">
              <Spinner />
            </div>
          }
        >
          <MermaidDiagram source={response.diagram} />
        </Suspense>
      ) : null}

      {response.sections?.map((section) =>
        isCollapsible(response.type, section) ? (
          <CollapsibleSection key={section.id} section={section} />
        ) : (
          <HeadedSection key={section.id} section={section} />
        ),
      )}

      {response.citations && response.citations.length > 0 ? (
        <div className="mt-4 border-t border-hud-border pt-3">
          <div className="mb-1.5 text-[11px] font-medium uppercase tracking-wide text-fg-subtle">Sources</div>
          <ol className="m-0 flex list-none flex-col gap-1 p-0">
            {response.citations.map((citation, index) => (
              <li key={citation.id} className="flex items-baseline gap-2 text-[13px]">
                <span className="text-fg-subtle">{index + 1}.</span>
                <button
                  type="button"
                  onClick={() => void openExternal(citation.url)}
                  className="inline-flex items-center gap-1 text-left text-accent hover:underline"
                >
                  {citation.title}
                  <ExternalLink className="size-3 shrink-0" aria-hidden />
                </button>
              </li>
            ))}
          </ol>
        </div>
      ) : null}
    </div>
  );
}
