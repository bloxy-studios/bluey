import { beforeEach, describe, expect, it } from "vitest";

import { MockTransport } from "@/lib/tauri/mock";

/** The mock's mode commands follow the Rust `ModeRepository` / `ModeManager`. */
describe("MockTransport modes", () => {
  let mock: MockTransport;

  beforeEach(() => {
    mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
  });

  const addFile = (modeId: string) =>
    mock.invoke("documents_add", {
      input: { kind: "notes", scope: "mode", scopeId: modeId, content: "agenda", title: "Agenda" },
    });
  const modeFiles = (modeId: string) => mock.invoke("documents_list", { scope: "mode", scopeId: modeId });

  it("creates with the Rust defaults and rejects what Rust rejects", async () => {
    const created = await mock.invoke("modes_create", { draft: { name: "  Pitch  " } });
    expect(created).toMatchObject({ name: "Pitch", contextRequirements: [], icon: "sparkles" });

    await expect(mock.invoke("modes_create", { draft: { name: " " } })).rejects.toMatchObject({
      code: "internal.invalid_params",
    });
    await expect(
      mock.invoke("modes_update", { id: created.id, patch: { systemInstructions: "a".repeat(4001) } }),
    ).rejects.toMatchObject({ code: "internal.invalid_params" });
    await expect(
      mock.invoke("modes_update", { id: created.id, patch: { preferredModelRole: "embedding" } }),
    ).rejects.toMatchObject({ code: "internal.invalid_params" });
  });

  it("keeps absent nullable fields and clears them on null", async () => {
    const { id } = await mock.invoke("modes_create", {
      draft: { name: "Pitch", group: "Work", preferredModelRole: "reasoning" },
    });

    const renamed = await mock.invoke("modes_update", { id, patch: { name: "Deal" } });
    expect(renamed).toMatchObject({ group: "Work", preferredModelRole: "reasoning" });

    const cleared = await mock.invoke("modes_update", { id, patch: { group: null, preferredModelRole: null } });
    expect(cleared).not.toHaveProperty("group");
    expect(cleared).not.toHaveProperty("preferredModelRole");
  });

  it("deleting the default mode resets the default and active mode and removes its files", async () => {
    const { id } = await mock.invoke("modes_create", { draft: { name: "Pitch" } });
    await addFile(id);
    await mock.invoke("modes_set_default", { id });
    expect((await mock.invoke("app_get_status", undefined)).modeId).toBe(id);

    await mock.invoke("modes_delete", { id });

    expect((await mock.invoke("settings_get", undefined)).general.defaultModeId).toBe("general");
    expect((await mock.invoke("app_get_status", undefined)).modeId).toBe("general");
    expect(await modeFiles(id)).toEqual([]);
  });

  it("a running session keeps its mode when the default changes", async () => {
    await mock.invoke("sessions_start", {});
    await mock.invoke("modes_set_default", { id: "sales" });

    expect((await mock.invoke("settings_get", undefined)).general.defaultModeId).toBe("sales");
    expect((await mock.invoke("app_get_status", undefined)).modeId).toBe("general");
  });

  it("duplicate copies the files to the new mode", async () => {
    const source = await addFile("sales");
    const copy = await mock.invoke("modes_duplicate", { id: "sales" });

    expect(copy.name).toBe("Sales (Copy)");
    const files = await modeFiles(copy.id);
    expect(files).toHaveLength(1);
    expect(files[0]?.id).not.toBe(source.id);
    expect(copy.attachedDocumentIds).toEqual([files[0]?.id]);
    expect(await modeFiles("sales")).toHaveLength(1);
  });
});
