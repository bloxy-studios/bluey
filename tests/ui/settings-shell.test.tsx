import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { SettingsShell } from "@/features/settings/SettingsShell";
import { useSettingsNav } from "@/features/settings/settings-nav";

const pages = vi.hoisted(() => ({ general: vi.fn(), appearance: vi.fn(), about: vi.fn() }));

vi.mock("@/features/settings/tabs/GeneralTab", () => ({
  default: function GeneralPage() {
    pages.general();
    const { setTab } = useSettingsNav();
    return <button onClick={() => setTab("about")}>General page — open About</button>;
  },
}));
vi.mock("@/features/settings/tabs/AppearanceTab", () => ({
  default: function AppearancePage() {
    pages.appearance();
    return <p>Appearance page</p>;
  },
}));
vi.mock("@/features/settings/tabs/AboutTab", () => ({
  default: function AboutPage() {
    pages.about();
    return <p>About page</p>;
  },
}));

const labels = [
  "General", "Appearance", "Modes", "Context", "Keybinds", "Audio", "Screen",
  "AI", "Privacy", "Permissions", "Sessions", "Profile", "Advanced", "About",
];

/** jsdom has no layout: model just the horizontal toolbar's measured geometry. */
function layoutToolbar(width = 650) {
  const list = screen.getByRole("tablist", { name: "Settings sections" });
  const viewport = list.parentElement!;
  const tabs = within(list).getAllByRole("tab");
  Object.defineProperties(viewport, {
    clientWidth: { configurable: true, value: width },
    scrollWidth: { configurable: true, value: 1150 },
  });
  vi.spyOn(viewport, "getBoundingClientRect").mockImplementation(() =>
    ({ left: 40, right: 40 + viewport.clientWidth }) as DOMRect,
  );
  tabs.forEach((tab, index) => {
    vi.spyOn(tab, "getBoundingClientRect").mockImplementation(() => ({
      left: 44 + index * 82 - viewport.scrollLeft,
      right: 120 + index * 82 - viewport.scrollLeft,
    }) as DOMRect);
  });
  fireEvent.scroll(viewport);
  return viewport;
}

describe("SettingsShell native toolbar", () => {
  beforeEach(() => {
    window.history.replaceState({}, "", "/?window=settings");
    vi.clearAllMocks();
  });
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    window.history.replaceState({}, "", "/");
  });

  it("keeps all 14 sections in order with linked tabs and one active panel", async () => {
    render(<SettingsShell />);
    await screen.findByText("General page — open About");
    const tabs = within(screen.getByRole("tablist")).getAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual(labels);
    expect(screen.getAllByRole("tabpanel")).toHaveLength(1);
    const activePanel = screen.getByRole("tabpanel", { name: "General" });
    for (const tab of tabs) {
      const panel = document.getElementById(tab.getAttribute("aria-controls")!);
      expect(panel).toHaveAttribute("role", "tabpanel");
      expect(panel).toHaveAttribute("aria-labelledby", tab.id);
      expect(tab).toHaveAttribute("aria-selected", String(tab.textContent === "General"));
    }
    expect(activePanel).toHaveAttribute("tabindex", "0");
    expect(pages.appearance).not.toHaveBeenCalled();
    expect(pages.about).not.toHaveBeenCalled();
    expect(screen.getByText("Bluey Settings")).toHaveClass("pointer-events-none");
    expect(screen.getByRole("tablist").closest("[data-tauri-drag-region]")).toBeNull();
  });

  it("roves with arrows / Home / End without loading or selecting until Enter or Space", async () => {
    const user = userEvent.setup();
    render(<SettingsShell />);
    await screen.findByText("General page — open About");
    const general = screen.getByRole("tab", { name: "General" });
    const appearance = screen.getByRole("tab", { name: "Appearance" });
    const about = screen.getByRole("tab", { name: "About" });
    await user.tab();
    expect(general).toHaveFocus();
    await user.keyboard("{ArrowRight}");
    await waitFor(() => expect(appearance).toHaveFocus());
    expect(general).toHaveAttribute("aria-selected", "true");
    expect(pages.appearance).not.toHaveBeenCalled();
    expect(within(screen.getByRole("tablist")).getAllByRole("tab").filter((tab) => tab.tabIndex === 0)).toEqual([appearance]);
    await user.keyboard("{End}");
    await waitFor(() => expect(about).toHaveFocus());
    expect(pages.about).not.toHaveBeenCalled();
    await user.keyboard("{ArrowRight}");
    await waitFor(() => expect(general).toHaveFocus());
    await user.keyboard("{ArrowLeft}");
    await waitFor(() => expect(about).toHaveFocus());
    await user.keyboard("{Home}");
    await waitFor(() => expect(general).toHaveFocus());
    await user.keyboard("{ArrowRight}");
    await waitFor(() => expect(appearance).toHaveFocus());
    await user.keyboard("{Enter}");
    await screen.findByText("Appearance page");
    expect(appearance).toHaveAttribute("aria-selected", "true");
    await user.keyboard("{End}");
    await waitFor(() => expect(about).toHaveFocus());
    await user.keyboard(" ");
    await screen.findByText("About page");
    expect(about).toHaveAttribute("aria-selected", "true");
  });

  it("can leave and re-enter a manually browsed toolbar in either Tab direction", async () => {
    const user = userEvent.setup();
    render(<><button>Before settings</button><SettingsShell /><button>After settings</button></>);
    await screen.findByText("General page — open About");
    const general = screen.getByRole("tab", { name: "General" });
    const appearance = screen.getByRole("tab", { name: "Appearance" });
    const panel = screen.getByRole("tabpanel", { name: "General" });
    await user.tab();
    expect(screen.getByRole("button", { name: "Before settings" })).toHaveFocus();
    await user.tab();
    expect(general).toHaveFocus();
    await user.keyboard("{ArrowRight}");
    await waitFor(() => expect(appearance).toHaveFocus());
    await user.tab();
    expect(panel).toHaveFocus();
    await user.tab({ shift: true });
    expect(appearance).toHaveFocus();
    await user.tab({ shift: true });
    expect(screen.getByRole("button", { name: "Before settings" })).toHaveFocus();
    await user.tab();
    expect(general).toHaveFocus();
    expect(general).toHaveAttribute("aria-selected", "true");
    expect(pages.appearance).not.toHaveBeenCalled();
  });

  it("starts deep links on their selected section and rejects unknown sections", async () => {
    window.history.replaceState({}, "", "/?window=settings&tab=about");
    const first = render(<SettingsShell />);
    await screen.findByText("About page");
    expect(screen.getByRole("tab", { name: "About" })).toHaveAttribute("aria-selected", "true");
    expect(pages.general).not.toHaveBeenCalled();
    first.unmount();
    window.history.replaceState({}, "", "/?window=settings&tab=missing");
    render(<SettingsShell />);
    await screen.findByText("General page — open About");
    expect(screen.getByRole("tab", { name: "General" })).toHaveAttribute("aria-selected", "true");
  });

  it("exposes overflow controls, bounds scrolling, and reveals keyboard focus without activation", async () => {
    const user = userEvent.setup();
    render(<SettingsShell />);
    await screen.findByText("General page — open About");
    const viewport = layoutToolbar();
    const earlier = screen.getByRole("button", { name: "Show earlier settings sections" });
    const later = screen.getByRole("button", { name: "Show later settings sections" });
    expect(earlier).toBeDisabled();
    expect(later).toBeEnabled();
    await user.click(later);
    expect(viewport.scrollLeft).toBeGreaterThan(0);
    await user.click(later);
    expect(viewport.scrollLeft).toBe(500);
    expect(later).toBeDisabled();
    await user.click(earlier);
    await user.click(earlier);
    expect(viewport.scrollLeft).toBe(0);
    act(() => screen.getByRole("tab", { name: "General" }).focus());
    await user.keyboard("{End}");
    await waitFor(() => expect(screen.getByRole("tab", { name: "About" })).toHaveFocus());
    expect(viewport.scrollLeft).toBe(500);
    expect(screen.getByRole("tabpanel", { name: "General" })).toBeInTheDocument();
    expect(pages.about).not.toHaveBeenCalled();
  });

  it("reveals programmatic selection and resets scroll without replacing linked panels", async () => {
    const user = userEvent.setup();
    render(<SettingsShell />);
    await screen.findByText("General page — open About");
    const viewport = layoutToolbar();
    const original = screen.getByRole("tabpanel", { name: "General" });
    const main = screen.getByRole("main");
    const linkedPanels = screen.getAllByRole("tab").map((tab) =>
      document.getElementById(tab.getAttribute("aria-controls")!),
    );
    original.scrollTop = 360;
    await user.click(screen.getByText("General page — open About"));
    await screen.findByText("About page");
    expect(viewport.scrollLeft).toBe(500);
    const about = screen.getByRole("tabpanel", { name: "About" });
    expect(about.scrollTop).toBe(0);
    expect(about).toHaveClass("overscroll-contain");
    expect(about).toHaveFocus();
    expect(screen.getByRole("main")).toBe(main);
    screen.getAllByRole("tab").forEach((tab, index) => {
      const panel = document.getElementById(tab.getAttribute("aria-controls")!);
      expect(panel).toBe(linkedPanels[index]);
      expect(panel).toHaveAttribute("aria-labelledby", tab.id);
    });
    await user.click(screen.getByRole("tab", { name: "General" }));
    await screen.findByText("General page — open About");
    const returned = screen.getByRole("tabpanel", { name: "General" });
    expect(returned).toBe(original);
    expect(returned.scrollTop).toBe(0);
  });

  it("reports wheel/trackpad edges and ignores rubber-band offsets when everything fits", async () => {
    render(<SettingsShell />);
    await screen.findByText("General page — open About");
    const viewport = layoutToolbar();
    const earlier = screen.getByRole("button", { name: "Show earlier settings sections" });
    const later = screen.getByRole("button", { name: "Show later settings sections" });
    viewport.scrollLeft = 250;
    fireEvent.scroll(viewport);
    expect(earlier).toBeEnabled();
    expect(later).toBeEnabled();
    viewport.scrollLeft = 499.5;
    fireEvent.scroll(viewport);
    expect(later).toBeDisabled();
    viewport.scrollLeft = -40;
    fireEvent.scroll(viewport);
    expect(earlier).toBeDisabled();
    expect(later).toBeEnabled();
    Object.defineProperty(viewport, "clientWidth", { configurable: true, value: 1150 });
    for (const bouncedOffset of [-40, 40]) {
      viewport.scrollLeft = bouncedOffset;
      fireEvent.scroll(viewport);
      expect(earlier).toBeDisabled();
      expect(later).toBeDisabled();
    }
  });

  it("rechecks overflow and selected visibility after resizing", async () => {
    let resize: (() => void) | undefined;
    const disconnect = vi.fn();
    vi.stubGlobal("ResizeObserver", class {
      constructor(callback: () => void) { resize = callback; }
      observe() {}
      disconnect = disconnect;
    });
    const user = userEvent.setup();
    const rendered = render(<SettingsShell />);
    await screen.findByText("General page — open About");
    const viewport = layoutToolbar(1150);
    await user.click(screen.getByText("General page — open About"));
    await screen.findByText("About page");
    Object.defineProperty(viewport, "clientWidth", { configurable: true, value: 650 });
    act(() => resize?.());
    expect(viewport.scrollLeft).toBe(500);
    expect(screen.getByRole("button", { name: "Show earlier settings sections" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Show later settings sections" })).toBeDisabled();
    rendered.unmount();
    expect(disconnect).toHaveBeenCalledOnce();
  });
});
