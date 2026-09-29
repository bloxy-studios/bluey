import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import PermissionsTab from "@/features/settings/tabs/PermissionsTab";
import { PERMISSIONS_POLL_MS } from "@/hooks/useLivePermissions";
import type { PermissionStatus } from "@/lib/types";
import { setupInterceptedApp } from "./helpers";

/** macOS flips a permission without telling Bluey; only `permissions_get` sees it. */
describe("live permission badges (ONB-002)", () => {
  let accessibility: PermissionStatus;
  let reads: number;

  beforeEach(async () => {
    accessibility = "denied";
    reads = 0;
    const { transport } = await setupInterceptedApp();
    transport.intercept("permissions_get", async (_args, next) => {
      reads += 1;
      return { ...(await next()), accessibility };
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  const accessibilityBadge = () =>
    screen.getByText("Accessibility").parentElement?.querySelector("span")?.textContent;

  it("re-reads permissions when the window regains focus", async () => {
    render(<PermissionsTab />);
    await waitFor(() => expect(accessibilityBadge()).toBe("Denied"));

    accessibility = "granted"; // granted in System Settings
    const before = reads;
    fireEvent.focus(window);

    await waitFor(() => expect(accessibilityBadge()).toBe("Granted"));
    expect(reads).toBeGreaterThan(before);
  });

  it("polls while the permissions screen is open, and stops once it closes", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const view = render(<PermissionsTab />);
    await waitFor(() => expect(accessibilityBadge()).toBe("Denied"));

    accessibility = "granted";
    await act(async () => {
      await vi.advanceTimersByTimeAsync(PERMISSIONS_POLL_MS);
    });
    await waitFor(() => expect(accessibilityBadge()).toBe("Granted"));

    view.unmount();
    const after = reads;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(PERMISSIONS_POLL_MS * 3);
    });
    expect(reads).toBe(after);
  });
});
