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

/** `rgba(r, g, b, a)` → colour and alpha. */
function rgba(color: string): [Rgb, number] {
  const parts = /rgba\(([^)]+)\)/.exec(color)?.[1]?.split(",").map((part) => Number(part.trim()));
  if (!parts || parts.length !== 4) throw new Error(`not an rgba colour: ${color}`);
  return [[parts[0], parts[1], parts[2]] as Rgb, parts[3] as number];
}

/** `color` at `alpha` over an opaque backdrop. */
function over([r, g, b]: Rgb, alpha: number, [br, bg, bb]: Rgb): Rgb {
  return [r * alpha + br * (1 - alpha), g * alpha + bg * (1 - alpha), b * alpha + bb * (1 - alpha)];
}

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

/** The default panel opacity (`appearance.opacity`, bluey-core settings). */
const DEFAULT_PANEL_OPACITY = 0.92;
const WHITE: Rgb = [255, 255, 255];
const BLACK: Rgb = [0, 0, 0];

describe("HUD text tokens (UX-014)", () => {
  it.each(["dark", "light"] as const)(
    "%s HUD text reads over a white or black backdrop at the default opacity",
    (theme) => {
      const [surface, alpha] = rgba(token(theme, "color-hud-bg"));
      for (const backdrop of [WHITE, BLACK]) {
        const background = over(surface, alpha * DEFAULT_PANEL_OPACITY, backdrop);
        const at = `${theme} over ${backdrop === WHITE ? "white" : "black"}`;
        expect(contrast(hex(token(theme, "color-fg")), background), `fg ${at}`).toBeGreaterThanOrEqual(4.5);
        expect(contrast(hex(token(theme, "color-hud-fg-muted")), background), `muted ${at}`).toBeGreaterThanOrEqual(3);
        expect(contrast(hex(token(theme, "color-hud-fg-subtle")), background), `subtle ${at}`).toBeGreaterThanOrEqual(3);
      }
    },
  );

  it("maps the opacity preference to the background only", () => {
    expect(CSS).toMatch(/\.hud-surface \{[^}]*--color-fg-muted: var\(--color-hud-fg-muted\)/);
    expect(CSS).toMatch(/\.hud-surface \{[^}]*background-color: color-mix\([^;]*var\(--hud-opacity/);
  });
});
