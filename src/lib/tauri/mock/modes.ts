/**
 * Mode semantics of the mock transport, kept faithful to the Rust
 * `ModeRepository` / `ModeManager`: create defaults, the same validation
 * limits (`internal.invalid_params`), and patches where an absent key keeps a
 * value while `null` clears `group` / `preferredModelRole` (cleared fields are
 * omitted, like Rust's `skip_serializing_if`).
 */

import { validateModeDraft } from "../../../modes/registry";
import type { BlueyError, BlueyMode, ModeDraft, ModePatch } from "../../types";
import { createId } from "../../utils/id";

export function invalidParams(message: string): BlueyError {
  return { kind: "internal", code: "internal.invalid_params", message, recoverable: false };
}

/** Throws `internal.invalid_params` for a draft/patch Rust would reject. */
export function assertValidModePatch(patch: ModePatch): void {
  const [firstError] = validateModeDraft({
    ...patch,
    name: patch.name ?? "mode",
    group: patch.group ?? undefined,
    preferredModelRole: patch.preferredModelRole ?? undefined,
  }).errors;
  if (firstError !== undefined) throw invalidParams(firstError);
}

/** A new custom mode from a draft (`ModeRepository::create`). */
export function createCustomMode(draft: ModeDraft, at: string): BlueyMode {
  assertValidModePatch(draft);
  const mode: BlueyMode = {
    id: createId("mode"),
    name: draft.name.trim(),
    description: draft.description ?? "",
    icon: draft.icon ?? "sparkles",
    systemInstructions: draft.systemInstructions ?? "",
    responseSchema: draft.responseSchema ?? "answer",
    preferredLatency: draft.preferredLatency ?? "fast",
    contextRequirements: draft.contextRequirements ?? [],
    builtIn: false,
    ...(draft.responseStyle ? { responseStyle: draft.responseStyle } : {}),
    attachedDocumentIds: [],
    createdAt: at,
    updatedAt: at,
  };
  return withNullables(mode, draft);
}

/** Apply a partial update (`ModeRepository::update`). */
export function applyModePatch(mode: BlueyMode, patch: ModePatch, at: string): BlueyMode {
  assertValidModePatch(patch);
  const { group, preferredModelRole, name, ...rest } = patch;
  const present = Object.fromEntries(Object.entries(rest).filter(([, value]) => value !== undefined));
  const updated: BlueyMode = {
    ...mode,
    ...present,
    ...(name !== undefined ? { name: name.trim() } : {}),
    updatedAt: at,
  };
  return withNullables(updated, { group, preferredModelRole });
}

/** Set or clear the nullable fields a patch names; absent keys keep them. */
function withNullables(mode: BlueyMode, patch: Pick<ModePatch, "group" | "preferredModelRole">): BlueyMode {
  const { group: currentGroup, preferredModelRole: currentRole, ...rest } = mode;
  const group = patch.group === undefined ? currentGroup : patch.group?.trim() || undefined;
  const role = patch.preferredModelRole === undefined ? currentRole : (patch.preferredModelRole ?? undefined);
  return {
    ...rest,
    ...(group ? { group } : {}),
    ...(role ? { preferredModelRole: role } : {}),
  };
}
