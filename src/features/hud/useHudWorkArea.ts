import { useEffect, useState } from "react";

import { hasTauriRuntime } from "@/lib/tauri/transport";
import { currentWorkArea, watchWorkAreaChanges, type WorkAreaSize } from "@/lib/tauri/work-area";
import { HUD_FRAME_INSETS, HUD_MAX_SURFACE_HEIGHT } from "./geometry";

function fallbackWorkArea(): WorkAreaSize {
  if (!hasTauriRuntime()) return { width: window.innerWidth, height: window.innerHeight };
  // Screen dimensions are independent of our auto-sized window. Using the
  // native innerHeight here would lock a collapsed panel out of growing.
  return {
    width: window.screen.availWidth || window.innerWidth,
    height:
      window.screen.availHeight || HUD_MAX_SURFACE_HEIGHT + HUD_FRAME_INSETS.top + HUD_FRAME_INSETS.bottom,
  };
}

/**
 * Native work-area dimensions only; do not invent a mixed-DPI global origin.
 * Tauri's Monitor.workArea.size is physical and is divided by THAT monitor's
 * scaleFactor once (not window.devicePixelRatio). Existing HUD capabilities
 * already grant current-monitor and window-event subscriptions.
 */
export function useHudWorkArea(): WorkAreaSize {
  const [work, setWork] = useState(fallbackWorkArea);

  useEffect(() => {
    let disposed = false;
    let revision = 0;
    const native = hasTauriRuntime();
    let screenHeight = window.screen.availHeight;
    let screenWidth = window.screen.availWidth;

    const refresh = async () => {
      if (disposed) return;
      const request = ++revision;
      let next = fallbackWorkArea();
      if (native) {
        try {
          next = (await currentWorkArea()) ?? next;
        } catch {
          // The independent screen fallback still permits growth if a monitor
          // was just detached or the platform cannot provide its work area.
        }
      }
      if (!disposed && request === revision) {
        setWork((previous) =>
          previous.width === next.width && previous.height === next.height ? previous : next,
        );
      }
    };
    const onResize = () => {
      // Streaming resizes do not require another monitor IPC. Refresh when
      // screen bounds change, or when resizing a browser preview viewport.
      if (!native || screenHeight !== window.screen.availHeight || screenWidth !== window.screen.availWidth) {
        screenHeight = window.screen.availHeight;
        screenWidth = window.screen.availWidth;
        void refresh();
      }
    };
    const onFocus = () => {
      void refresh();
    };
    window.addEventListener("resize", onResize);
    window.addEventListener("focus", onFocus);

    const unlisten = watchWorkAreaChanges(() => {
      void refresh();
    });
    void refresh();

    return () => {
      disposed = true;
      window.removeEventListener("resize", onResize);
      window.removeEventListener("focus", onFocus);
      unlisten();
    };
  }, []);

  return work;
}
