/**
 * Lazy, fine-grained shiki highlighter. Core + JS regex engine + the two
 * themes load on first use; each language grammar loads on demand. Nothing
 * lands in the initial bundle.
 */

import type { HighlighterCore } from "shiki/core";

export const SUPPORTED_LANGUAGES = [
  "python",
  "javascript",
  "typescript",
  "java",
  "cpp",
  "csharp",
  "go",
  "rust",
  "sql",
  "bash",
  "json",
] as const;

export type SupportedLanguage = (typeof SUPPORTED_LANGUAGES)[number];

const LANGUAGE_ALIASES: Record<string, SupportedLanguage> = {
  py: "python",
  python3: "python",
  js: "javascript",
  jsx: "javascript",
  mjs: "javascript",
  node: "javascript",
  ts: "typescript",
  tsx: "typescript",
  "c++": "cpp",
  cc: "cpp",
  c: "cpp",
  cs: "csharp",
  "c#": "csharp",
  golang: "go",
  rs: "rust",
  sh: "bash",
  shell: "bash",
  zsh: "bash",
  console: "bash",
  jsonc: "json",
};

const LANGUAGE_LOADERS: Record<SupportedLanguage, () => Promise<{ default: unknown }>> = {
  python: () => import("shiki/dist/langs/python.mjs"),
  javascript: () => import("shiki/dist/langs/javascript.mjs"),
  typescript: () => import("shiki/dist/langs/typescript.mjs"),
  java: () => import("shiki/dist/langs/java.mjs"),
  cpp: () => import("shiki/dist/langs/cpp.mjs"),
  csharp: () => import("shiki/dist/langs/csharp.mjs"),
  go: () => import("shiki/dist/langs/go.mjs"),
  rust: () => import("shiki/dist/langs/rust.mjs"),
  sql: () => import("shiki/dist/langs/sql.mjs"),
  bash: () => import("shiki/dist/langs/bash.mjs"),
  json: () => import("shiki/dist/langs/json.mjs"),
};

export function resolveLanguage(language: string | undefined): SupportedLanguage | null {
  if (!language) return null;
  const normalized = language.toLowerCase();
  if ((SUPPORTED_LANGUAGES as readonly string[]).includes(normalized)) return normalized as SupportedLanguage;
  return LANGUAGE_ALIASES[normalized] ?? null;
}

let corePromise: Promise<HighlighterCore> | null = null;
const loadedLanguages = new Set<SupportedLanguage>();

async function getCore(): Promise<HighlighterCore> {
  if (!corePromise) {
    corePromise = (async () => {
      const [{ createHighlighterCore }, { createJavaScriptRegexEngine }, darkTheme, lightTheme] = await Promise.all([
        import("shiki/core"),
        import("shiki/engine/javascript"),
        import("shiki/dist/themes/github-dark-default.mjs"),
        import("shiki/dist/themes/github-light-default.mjs"),
      ]);
      return createHighlighterCore({
        themes: [darkTheme.default, lightTheme.default],
        langs: [],
        engine: createJavaScriptRegexEngine({ forgiving: true }),
      });
    })();
  }
  return corePromise;
}

/** Highlight code to HTML; returns null when the language is unsupported or loading fails. */
export async function highlightCode(code: string, language: string | undefined, theme: "dark" | "light"): Promise<string | null> {
  const lang = resolveLanguage(language);
  if (!lang) return null;
  try {
    const core = await getCore();
    if (!loadedLanguages.has(lang)) {
      const grammar = await LANGUAGE_LOADERS[lang]();
      await core.loadLanguage(grammar.default as Parameters<HighlighterCore["loadLanguage"]>[0]);
      loadedLanguages.add(lang);
    }
    return core.codeToHtml(code, {
      lang,
      theme: theme === "dark" ? "github-dark-default" : "github-light-default",
    });
  } catch (error) {
    console.warn("[highlighter] failed", error);
    return null;
  }
}
