/**
 * Command-surface parity: every command declared in the TypeScript contract
 * (`COMMAND_NAMES`) must be registered in the Rust `generate_handler![...]`
 * list, and vice versa. Also checks that every event name in `EVENT_NAMES`
 * appears in the Rust `BlueyEvent::name()` implementation.
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

import { COMMAND_NAMES } from "@/lib/tauri/commands";
import { EVENT_NAMES, tauriEventName } from "@/lib/tauri/events";

const ROOT = resolve(__dirname, "../..");

function rustHandlerNames(): string[] {
  const source = readFileSync(resolve(ROOT, "src-tauri/src/lib.rs"), "utf8");
  const start = source.indexOf("generate_handler!");
  if (start < 0) return [];
  const open = source.indexOf("[", start);
  let depth = 0;
  let end = open;
  for (let i = open; i < source.length; i++) {
    const ch = source[i];
    if (ch === "[") depth += 1;
    if (ch === "]") {
      depth -= 1;
      if (depth === 0) {
        end = i;
        break;
      }
    }
  }
  const body = source.slice(open + 1, end).replace(/\/\/.*$/gm, "");
  return body
    .split(",")
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0)
    .map((entry) => entry.split("::").pop() ?? entry)
    .map((entry) => entry.trim());
}

describe("command surface parity (TS contract ⇄ Rust generate_handler!)", () => {
  const rust = rustHandlerNames();

  it("Rust registers every command declared in commands.ts", () => {
    const missing = COMMAND_NAMES.filter((name) => !rust.includes(name));
    expect(missing, `commands missing from generate_handler!: ${missing.join(", ")}`).toEqual([]);
  });

  it("commands.ts declares every command Rust registers", () => {
    const extra = rust.filter((name) => !(COMMAND_NAMES as readonly string[]).includes(name));
    expect(extra, `commands registered in Rust but absent from commands.ts: ${extra.join(", ")}`).toEqual([]);
  });

  it("has no duplicate command names on either side", () => {
    expect(new Set(COMMAND_NAMES).size).toBe(COMMAND_NAMES.length);
    expect(new Set(rust).size).toBe(rust.length);
  });
});

describe("event surface parity (events.ts ⇄ bluey_core::events)", () => {
  const eventsDir = resolve(ROOT, "src-tauri/crates/bluey-core/src/events");

  it("every TS event name appears in the Rust event module", () => {
    const source = readFileSync(resolve(eventsDir, "mod.rs"), "utf8");
    const missing = EVENT_NAMES.filter((name) => !source.includes(`"${name}"`));
    expect(missing, `event names missing in Rust: ${missing.join(", ")}`).toEqual([]);
  });

  it("the Rust test copy of EVENT_NAMES matches events.ts exactly (same order)", () => {
    const source = readFileSync(resolve(eventsDir, "tests.rs"), "utf8");
    const body = /const EVENT_NAMES: \[&str; \d+\] = \[([\s\S]*?)\];/.exec(source)?.[1];
    expect(body, "EVENT_NAMES array not found in tests.rs").toBeDefined();
    const rust = [...(body ?? "").matchAll(/"([^"]+)"/g)].map((m) => m[1] ?? "");
    expect(rust).toEqual([...EVENT_NAMES]);
  });

  // Tauri v2 rejects event names outside [A-Za-z0-9-/:_]: a dot in a wire name
  // means `emit` fails and the WebView never hears the event.
  it("maps every event to a name Tauri accepts", () => {
    const allowed = /^[A-Za-z0-9\-/:_]+$/;
    const rejected = EVENT_NAMES.map(tauriEventName).filter((wire) => !allowed.test(wire));
    expect(rejected, `wire names Tauri would reject: ${rejected.join(", ")}`).toEqual([]);
    expect(tauriEventName("panel.newChat")).toBe("bluey:panel/newChat");
    expect(tauriEventName("audio.deviceChanged")).toBe("bluey:audio/deviceChanged");
  });
});
