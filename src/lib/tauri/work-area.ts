import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";

import { hasTauriRuntime, type Unlisten } from "./transport";

export interface WorkAreaSize {
  width: number;
  height: number;
}

/**
 * The window/monitor API stays at the Tauri boundary. Only dimensions cross
 * into UI code; do not manufacture a mixed-DPI global origin. Work-area sizes
 * are physical and must be divided by THAT monitor's scale exactly once.
 */
export async function currentWorkArea(): Promise<WorkAreaSize | null> {
  if (!hasTauriRuntime()) return null;
  const monitor = await currentMonitor();
  if (!monitor || !Number.isFinite(monitor.scaleFactor) || monitor.scaleFactor <= 0) return null;
  const size = monitor.workArea.size.toLogical(monitor.scaleFactor);
  return Number.isFinite(size.width) && size.width > 0 && Number.isFinite(size.height) && size.height > 0
    ? { width: size.width, height: size.height }
    : null;
}

/** Dispose both registered and late-arriving native listeners on unmount. */
export function watchWorkAreaChanges(refresh: () => void): Unlisten {
  if (!hasTauriRuntime()) return () => {};
  let disposed = false;
  const unlisteners: Unlisten[] = [];
  const notify = () => {
    if (!disposed) refresh();
  };
  const retain = (subscription: Promise<Unlisten>) => {
    void subscription
      .then((unlisten) => {
        if (disposed) unlisten();
        else unlisteners.push(unlisten);
      })
      .catch(() => undefined);
  };
  const current = getCurrentWindow();
  retain(current.onMoved(notify));
  retain(current.onScaleChanged(notify));
  return () => {
    disposed = true;
    unlisteners.splice(0).forEach((unlisten) => unlisten());
  };
}
