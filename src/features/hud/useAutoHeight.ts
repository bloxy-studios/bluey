import { useEffect, type RefObject } from "react";

import { debounce } from "@/lib/utils/debounce";
import { usePanelStore } from "@/stores/panelStore";

/**
 * Measures the panel's rendered height with a ResizeObserver and reports it
 * (debounced) to the backend via `bluey.panel.setExpanded` so the native
 * NSPanel can resize to fit.
 */
export function useAutoHeight(ref: RefObject<HTMLElement | null>, expanded: boolean): void {
  useEffect(() => {
    const element = ref.current;
    if (!element || typeof ResizeObserver === "undefined") return;

    const report = debounce((height: number) => {
      void usePanelStore.getState().setExpanded(expanded, Math.ceil(height));
    }, 80);

    const observer = new ResizeObserver((entries) => {
      const height = entries[0]?.contentRect.height;
      if (height && height > 0) report(height);
    });
    observer.observe(element);
    report(element.getBoundingClientRect().height);

    return () => {
      observer.disconnect();
      report.cancel();
    };
  }, [ref, expanded]);
}
