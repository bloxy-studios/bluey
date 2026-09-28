/**
 * A failed screen capture degrades the snapshot instead of failing the ask
 * (CTX-010): the mock mirrors Rust's `build_snapshot` — no screen and no OCR,
 * the accessibility tree and app identity kept, and a `screen_unavailable`
 * warning carrying the permission error's code and recovery.
 */

import { MockTransport } from "@/lib/tauri/mock/mock-transport";
import type { ContextSnapshot } from "@/lib/types";

const options = {
  includeScreen: true,
  includeOcr: true,
  includeAccessibility: true,
  includeTranscript: false,
  inlineImage: true,
};

describe("snapshot without Screen Recording", () => {
  it("keeps accessibility and app identity and says why the screen is missing", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    await mock.invoke("dev_simulate", { simulation: { type: "permission_error", permission: "screenRecording" } });

    const snapshot: ContextSnapshot = await mock.invoke("context_build_snapshot", { options });

    expect(snapshot.screen).toBeUndefined();
    expect(snapshot.ocr).toBeUndefined();
    expect(snapshot.accessibility?.visibleText).toContain("Two Sum");
    expect(snapshot.activeApplication?.name).toBe("Google Chrome");
    expect(snapshot.warnings).toEqual([
      expect.objectContaining({
        kind: "screen_unavailable",
        code: "permission.screen_recording",
        recovery: { type: "open_system_settings", pane: "screenRecording" },
      }),
    ]);
  });

  it("carries no warnings when the capture succeeds", async () => {
    const mock = new MockTransport({ streamDelayMs: 0, levelTicks: false });
    const snapshot: ContextSnapshot = await mock.invoke("context_build_snapshot", { options });
    expect(snapshot.screen?.image).toBeTruthy();
    expect(snapshot.warnings).toBeUndefined();
  });
});
