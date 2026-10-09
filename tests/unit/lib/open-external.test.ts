import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { useToastStore } from "@/components/ui/toast-store";
import { openExternal } from "@/lib/utils/open-external";

const CAPABILITIES_DIR = join(__dirname, "../../../src-tauri/capabilities");

type Permission = string | { identifier: string; allow?: Array<{ url?: string }> };

describe("opener capabilities (UX-006)", () => {
  const files = readdirSync(CAPABILITIES_DIR).filter((f) => f.endsWith(".json"));

  it.each(files)("%s scopes opener:allow-open-url to web and email links", (file) => {
    const capability = JSON.parse(readFileSync(join(CAPABILITIES_DIR, file), "utf8")) as {
      permissions: Permission[];
    };
    const opener = capability.permissions.filter((p) =>
      (typeof p === "string" ? p : p.identifier).startsWith("opener:"),
    );
    if (opener.length === 0) return;
    // A bare "opener:allow-open-url" has an empty scope: every openUrl is refused.
    expect(opener).not.toContain("opener:allow-open-url");
    const urls = opener.flatMap((p) => (typeof p === "string" ? [] : (p.allow ?? []).map((a) => a.url)));
    expect(urls).toEqual(expect.arrayContaining(["https://*", "http://*", "mailto:*"]));
    expect(urls.every((url) => /^(https?:\/\/|mailto:)/.test(url ?? ""))).toBe(true);
  });
});

describe("openExternal", () => {
  beforeEach(() => {
    useToastStore.setState({ toasts: [] });
  });

  it.each(["javascript:alert(1)", "file:///etc/passwd", "not a url"])(
    "refuses %s with a toast",
    async (url) => {
      const open = vi.spyOn(window, "open").mockReturnValue(null);
      await openExternal(url);
      expect(open).not.toHaveBeenCalled();
      expect(useToastStore.getState().toasts.map((t) => t.title)).toContain("Can't open this link");
      open.mockRestore();
    },
  );

  it("opens web links and surfaces a failure instead of swallowing it", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    await openExternal("https://example.com/a");
    expect(open).toHaveBeenCalledWith("https://example.com/a", "_blank", "noopener,noreferrer");

    open.mockImplementation(() => {
      throw new Error("blocked");
    });
    await openExternal("mailto:support@bluey.app");
    expect(useToastStore.getState().toasts.map((t) => t.title)).toContain("Couldn't open the link");
    open.mockRestore();
  });
});
