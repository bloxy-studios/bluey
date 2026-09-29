import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

import darkShiki from "shiki/dist/themes/github-dark-default.mjs";
import lightShiki from "shiki/dist/themes/github-light-default.mjs";

const CSS = readFileSync(resolve(__dirname, "../../src/app/styles/theme.css"), "utf8");

/** The declarations of one CSS block: `@theme` (dark, the default) or the light override. */
function block(selector: "@theme" | ':root[data-theme="light"]'): string {
  const start = CSS.indexOf(`${selector} {`);
  expect(start).toBeGreaterThanOrEqual(0);
  return CSS.slice(start, CSS.indexOf("\n}", start));
}

function token(theme: "dark" | "light", name: string): string {
  const source = block(theme === "dark" ? "@theme" : ':root[data-theme="light"]');
  const match = new RegExp(`--${name}:\\s*([^;]+);`).exec(source);
  if (!match?.[1]) throw new Error(`--${name} is not defined for the ${theme} theme`);
  return match[1].trim();
}

type Rgb = [number, number, number];

function hex(color: string): Rgb {
  const value = color.replace("#", "");
  return [0, 2, 4].map((i) => parseInt(value.slice(i, i + 2), 16)) as Rgb;
}

function luminance([r, g, b]: Rgb): number {
  const [lr, lg, lb] = [r, g, b].map((channel) => {
    const c = channel / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }) as Rgb;
  return 0.2126 * lr + 0.7152 * lg + 0.0722 * lb;
}

function contrast(a: Rgb, b: Rgb): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

/** Every foreground the shiki theme paints on the code background (rules with their own background excluded). */
function shikiForegrounds(theme: typeof lightShiki): string[] {
  const rules = (theme.tokenColors ?? theme.settings ?? []) as Array<{
    settings?: { foreground?: string; background?: string };
  }>;
  const colours = rules
    .filter((rule) => !rule.settings?.background)
    .map((rule) => rule.settings?.foreground)
    .filter((c): c is string => /^#[0-9a-f]{6}$/i.test(c ?? ""));
  return [theme.colors?.["editor.foreground"] as string, ...colours];
}

describe("code surface tokens (UX-005)", () => {
  it.each([
    ["dark", darkShiki],
    ["light", lightShiki],
  ] as const)("every %s shiki token colour reads at 4.5:1 on --color-code-bg", (theme, shiki) => {
    const background = hex(token(theme, "color-code-bg"));
    for (const foreground of shikiForegrounds(shiki)) {
      expect(contrast(hex(foreground), background), `${foreground} on ${theme}`).toBeGreaterThanOrEqual(4.5);
    }
  });
});
