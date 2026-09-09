/**
 * Real browser regressions (no native Tauri runtime is simulated).
 * Bun only. Supply PLAYWRIGHT_MODULE if Playwright lives in a separate QA install:
 * PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs bun tests/browser/hud-interactions.mjs
 * Does not install dependencies, change package manifests, or write screenshots.
 */
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("../../", import.meta.url));
const port = Number(process.env.HUD_TEST_PORT || 5188);
const base = `http://127.0.0.1:${port}`;
const server = Bun.spawn(
  [process.execPath, "--bun", "run", "dev", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  {
    cwd: root,
    stdout: "ignore",
    stderr: "ignore",
  },
);
let browser;
let passed = 0;
const failures = [];
async function check(name, run) {
  try {
    await run();
    passed++;
    console.info(`PASS ${name}`);
  } catch (error) {
    failures.push(name);
    console.error(`FAIL ${name}: ${error.stack || error}`);
  }
}
try {
  let ready = false;
  for (let i = 0; i < 80; i++) {
    if (server.exitCode !== null) throw new Error(`Vite exited: ${server.exitCode}`);
    try {
      if ((await fetch(base)).ok) {
        ready = true;
        break;
      }
    } catch {}
    await Bun.sleep(250);
  }
  assert.ok(ready, "Vite must start");
  browser = await chromium.launch({ headless: true, args: ["--no-sandbox"] });
  const page = await browser.newPage({ viewport: { width: 1024, height: 768 }, colorScheme: "dark" });
  page.setDefaultTimeout(5000);
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`${base}/?window=main`);
  await page.getByRole("button", { name: "Mode: General" }).waitFor();
  const modeTrigger = () => page.getByRole("button", { name: /^Mode: /, includeHidden: true });
  const sessionTrigger = () => page.getByRole("button", { name: /^Session(:| menu)/, includeHidden: true });
  const menu = () => page.getByRole("menu");
  async function open(trigger, key = "ArrowDown") {
    await trigger.focus();
    await page.keyboard.press(key);
    await menu().waitFor();
  }
  async function close() {
    await page.keyboard.press("Escape");
    await menu().waitFor({ state: "hidden" });
  }
  async function state() {
    return page.evaluate(async () => {
      const [{ useAppStore }, { useSessionStore }, { useToastStore }] = await Promise.all([
        import("/src/stores/appStore.ts"),
        import("/src/stores/sessionStore.ts"),
        import("/src/components/ui/toast-store.ts"),
      ]);
      return {
        mode: useAppStore.getState().status?.modeId,
        session: useSessionStore.getState().active,
        toasts: useToastStore.getState().toasts,
      };
    });
  }
  async function waitState(predicate) {
    for (let i = 0; i < 60; i++) {
      if (predicate(await state())) return;
      await Bun.sleep(50);
    }
    throw new Error(`State did not settle: ${JSON.stringify(await state())}`);
  }

  await check("actual toolbar buttons own menu semantics", async () => {
    for (const trigger of [modeTrigger(), sessionTrigger()]) {
      assert.equal(await trigger.getAttribute("aria-haspopup"), "menu");
      assert.equal(await trigger.getAttribute("aria-expanded"), "false");
      assert.equal(await trigger.evaluate((node) => node.tagName), "BUTTON");
      assert.notEqual(await trigger.evaluate((node) => node.parentElement.tagName), "SPAN");
    }
  });
  for (const key of ["Enter", "Space", "ArrowDown"]) {
    for (const [name, trigger] of [
      ["mode", modeTrigger],
      ["session", sessionTrigger],
    ]) {
      await check(`${name}: ${key} opens; Escape closes and restores focus`, async () => {
        await open(trigger(), key);
        assert.equal(await trigger().getAttribute("aria-expanded"), "true");
        await close();
        assert.equal(await trigger().getAttribute("aria-expanded"), "false");
        assert.ok(await trigger().evaluate((node) => node === document.activeElement));
      });
    }
  }
  await check("pointer opening, checked mode selection, and Manage preserve action parity", async () => {
    await modeTrigger().click();
    assert.equal(
      await page.getByRole("menuitemcheckbox", { name: "General", exact: true }).getAttribute("aria-checked"),
      "true",
    );
    await page.getByRole("menuitemcheckbox", { name: "Interview", exact: true }).click();
    await waitState((s) => s.mode === "interview");
    await open(modeTrigger());
    assert.equal(
      await page
        .getByRole("menuitemcheckbox", { name: "Interview", exact: true })
        .getAttribute("aria-checked"),
      "true",
    );
    await page.evaluate(async () => {
      const { bluey } = await import("/src/lib/tauri/api.ts");
      window.__hudOpenCalls = [];
      const original = bluey.window.open;
      bluey.window.open = (args) => {
        window.__hudOpenCalls.push(args);
        return original(args);
      };
    });
    await page.getByRole("menuitem", { name: "Manage", exact: true }).click();
    assert.deepEqual(await page.evaluate(() => window.__hudOpenCalls), [
      { label: "settings", route: "modes" },
    ]);
  });
  await check(
    "session start/pause/resume/History/end use existing guarded actions including audio",
    async () => {
      await page.evaluate(async () => {
        const { bluey } = await import("/src/lib/tauri/api.ts");
        await bluey.audio.start();
      });
      await open(sessionTrigger());
      await page.getByRole("menuitem", { name: "Start session" }).click();
      await waitState((s) => s.session?.status === "active");
      await open(sessionTrigger());
      await page.getByRole("menuitem", { name: "Pause session" }).click();
      await waitState((s) => s.session?.status === "paused");
      assert.equal(
        await page.evaluate(async () => {
          const { bluey } = await import("/src/lib/tauri/api.ts");
          return (await bluey.audio.getStatus()).state;
        }),
        "paused",
      );
      await open(sessionTrigger());
      assert.match(await menu().innerText(), /paused/);
      await page.getByRole("menuitem", { name: "Resume session" }).click();
      await waitState((s) => s.session?.status === "active");
      assert.equal(
        await page.evaluate(async () => {
          const { bluey } = await import("/src/lib/tauri/api.ts");
          return (await bluey.audio.getStatus()).state;
        }),
        "running",
      );
      await open(sessionTrigger());
      await page.getByRole("menuitem", { name: "Open in History" }).click();
      assert.deepEqual((await page.evaluate(() => window.__hudOpenCalls)).at(-1), {
        label: "settings",
        route: "sessions",
      });
      await open(sessionTrigger());
      await page.getByRole("menuitem", { name: "End session" }).click();
      await waitState((s) => s.session === null);
      await open(sessionTrigger());
      await page.getByRole("menuitem", { name: "Open History", exact: true }).click();
      assert.deepEqual((await page.evaluate(() => window.__hudOpenCalls)).at(-1), {
        label: "settings",
        route: "sessions",
      });
      await page.evaluate(async () => {
        const { bluey } = await import("/src/lib/tauri/api.ts");
        await bluey.audio.stop();
      });
    },
  );
  await check("failed pause keeps the session/audio active and uses the existing error toast", async () => {
    await page.evaluate(async () => {
      const { bluey } = await import("/src/lib/tauri/api.ts");
      await bluey.audio.start();
      await bluey.session.start({ title: "Failure check" });
      window.__hudOriginalPause = bluey.session.pause;
      bluey.session.pause = async () => {
        throw new Error("Pause rejected for test");
      };
    });
    await open(sessionTrigger());
    await page.getByRole("menuitem", { name: "Pause session" }).click();
    await waitState((s) => s.toasts.some((toast) => toast.variant === "error"));
    assert.equal((await state()).session?.status, "active");
    assert.equal(
      await page.evaluate(async () => {
        const { bluey } = await import("/src/lib/tauri/api.ts");
        return (await bluey.audio.getStatus()).state;
      }),
      "running",
    );
    await page.evaluate(async () => {
      const { bluey } = await import("/src/lib/tauri/api.ts");
      const { useToastStore } = await import("/src/components/ui/toast-store.ts");
      bluey.session.pause = window.__hudOriginalPause;
      await bluey.session.end();
      await bluey.audio.stop();
      for (const toast of useToastStore.getState().toasts) useToastStore.getState().dismiss(toast.id);
    });
  });
  await check("rejected mode actions surface a toast and close the web menu", async () => {
    await page.evaluate(async () => {
      const { bluey } = await import("/src/lib/tauri/api.ts");
      window.__hudOriginalSetActive = bluey.modes.setActive;
      bluey.modes.setActive = async () => {
        throw new Error("Menu test rejection");
      };
    });
    await open(modeTrigger());
    await page.getByRole("menuitemcheckbox", { name: "General", exact: true }).click();
    await waitState((s) => s.toasts.some((toast) => toast.variant === "error"));
    await menu().waitFor({ state: "hidden" });
    await page.evaluate(async () => {
      const { bluey } = await import("/src/lib/tauri/api.ts");
      bluey.modes.setActive = window.__hudOriginalSetActive;
    });
  });
  await check("twenty open/cancel cycles leave one menu while open and no portal/inert residue", async () => {
    for (let i = 0; i < 20; i++) {
      await open(modeTrigger());
      assert.equal(await menu().count(), 1);
      await close();
      assert.equal(await menu().count(), 0);
    }
    assert.notEqual(await page.locator("body").evaluate((node) => node.style.pointerEvents), "none");
  });
  await check("web focus restoration does not run when document.hasFocus is false", async () => {
    await open(modeTrigger());
    await page.evaluate(() => {
      window.__hudHasFocus = document.hasFocus;
      document.hasFocus = () => false;
      const trigger = document.querySelector('button[aria-label^="Mode: "]');
      window.__hudTriggerFocus = trigger.focus;
      window.__hudFocusCalls = 0;
      trigger.focus = () => {
        window.__hudFocusCalls++;
      };
    });
    await close();
    assert.equal(await page.evaluate(() => window.__hudFocusCalls), 0);
    await page.evaluate(() => {
      document.hasFocus = window.__hudHasFocus;
      document.querySelector('button[aria-label^="Mode: "]').focus = window.__hudTriggerFocus;
    });
  });
  await check("Escape used by a menu does not erase an existing chat", async () => {
    await page.evaluate(async () => {
      const { useChatStore } = await import("/src/stores/chatStore.ts");
      const generation = useChatStore.getState().begin("Keep this chat");
      useChatStore.getState().markCancelled(generation);
    });
    await open(modeTrigger());
    await close();
    assert.equal(
      await page.evaluate(
        async () => (await import("/src/stores/chatStore.ts")).useChatStore.getState().turns.length,
      ),
      1,
    );
    await page.getByRole("textbox", { name: "Ask follow-up" }).focus();
    await page.getByRole("tooltip").waitFor({ state: "hidden" });
    await page.keyboard.press("Escape");
    await page.getByRole("textbox", { name: "Ask Bluey" }).waitFor();
    assert.equal(
      await page.evaluate(
        async () => (await import("/src/stores/chatStore.ts")).useChatStore.getState().turns.length,
      ),
      0,
    );
  });
  await check("long menus/labels stay bounded and scroll in both light and dark themes", async () => {
    await page.evaluate(async () => {
      const { useModesStore } = await import("/src/stores/modesStore.ts");
      const { useAppStore } = await import("/src/stores/appStore.ts");
      const baseMode = useModesStore.getState().modes[0];
      window.__hudModeName = "VeryLongUnbrokenModeName".repeat(18);
      useModesStore.setState({
        modes: [
          { ...baseMode, name: window.__hudModeName },
          ...Array.from({ length: 24 }, (_, i) => ({
            ...baseMode,
            id: `display-test-${i}`,
            name: `Long mode ${i}`,
          })),
        ],
      });
      useAppStore.setState({ status: { ...useAppStore.getState().status, modeId: baseMode.id } });
    });
    await page.setViewportSize({ width: 700, height: 210 });
    for (const [theme, color] of [
      ["dark", "rgb(22, 22, 22)"],
      ["light", "rgb(255, 255, 255)"],
    ]) {
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await open(modeTrigger());
      const metrics = await menu().evaluate((node) => {
        const box = node.getBoundingClientRect();
        const style = getComputedStyle(node);
        return {
          x: box.x,
          right: box.right,
          bottom: box.bottom,
          height: box.height,
          scroll: node.scrollHeight,
          client: node.clientHeight,
          background: style.backgroundColor,
          overflow: style.overflowY,
        };
      });
      assert.equal(metrics.background, color);
      assert.equal(metrics.overflow, "auto");
      assert.ok(metrics.x >= 0 && metrics.right <= 700 && metrics.bottom <= 210);
      assert.ok(metrics.scroll > metrics.client);
      const label = await page.getByRole("menuitemcheckbox").first().innerText();
      assert.equal(Array.from(label).length, 80);
      await close();
    }
  });
  await check("long tooltip text wraps within the viewport", async () => {
    await modeTrigger().hover();
    const tooltip = page.getByRole("tooltip");
    await tooltip.waitFor();
    const metric = await tooltip.evaluate((node) => {
      const visible = node.closest("[data-radix-popper-content-wrapper]");
      const box = visible.getBoundingClientRect();
      return {
        width: box.width,
        right: box.right,
        x: box.x,
        scroll: visible.scrollWidth,
        client: visible.clientWidth,
      };
    });
    assert.ok(metric.width <= 280 && metric.x >= 0 && metric.right <= 700);
    assert.ok(metric.scroll <= metric.client + 1);
  });
  await check("compact toolbar keeps error recovery and every control in separate hit targets", async () => {
    await page.setViewportSize({ width: 1024, height: 768 });
    for (const width of [420, 520, 690]) {
      await page.evaluate(async (width) => {
        document.querySelector('[role="dialog"][aria-label="Bluey"]').style.width = `${width}px`;
        const { useAppStore } = await import("/src/stores/appStore.ts");
        useAppStore.setState({
          status: {
            ...useAppStore.getState().status,
            state: "error",
            error: {
              kind: "configuration",
              code: "config.missing_provider",
              message: "Provider setup needed",
              recoverable: true,
              recovery: { type: "open_settings", tab: "ai" },
            },
          },
        });
      }, width);
      await page.getByRole("button", { name: "Open Settings", exact: true }).waitFor();
      const hits = await page.evaluate(() => {
        const toolbar = document.querySelector(".hud-toolbar");
        const bounds = toolbar.getBoundingClientRect();
        const buttons = [...toolbar.querySelectorAll("button")]
          .map((button) => {
            const r = button.getBoundingClientRect();
            return {
              name: button.getAttribute("aria-label") || button.textContent,
              x: r.x,
              y: r.y,
              right: r.right,
              bottom: r.bottom,
              width: r.width,
              height: r.height,
            };
          })
          .filter((button) => button.width > 0 && button.height > 0);
        return {
          count: buttons.length,
          outside: buttons.filter((b) => b.x < bounds.x || b.right > bounds.right),
          overlaps: buttons.flatMap((a, i) =>
            buttons
              .slice(i + 1)
              .filter(
                (b) =>
                  Math.min(a.right, b.right) - Math.max(a.x, b.x) > 1 &&
                  Math.min(a.bottom, b.bottom) - Math.max(a.y, b.y) > 1,
              )
              .map((b) => [a.name, b.name]),
          ),
        };
      });
      assert.ok(hits.count >= 8, `all toolbar/recovery actions survive at ${width}px`);
      assert.deepEqual(hits.outside, [], `buttons inside toolbar at ${width}px`);
      assert.deepEqual(hits.overlaps, [], `separate hit targets at ${width}px`);
    }
  });
  await check("no browser page errors", async () => {
    assert.deepEqual(errors, []);
  });
  console.info(`Browser checks: ${passed} passed, ${failures.length} failed`);
  if (failures.length) process.exitCode = 1;
} finally {
  await browser?.close();
  server.kill();
  await server.exited;
}
