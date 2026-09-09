import { useEffect, useRef, type RefObject } from "react";

import { usePanelStore } from "@/stores/panelStore";
import { measuredFrameHeight } from "./geometry";

/**
 * Measure the content-sized outer frame, not a 100vh wrapper or a contentRect.
 * Coalesce at most one update per 80ms without starving a continuous stream.
 * The store serializes native requests; identical measurements are no-ops.
 */
export function useAutoHeight(ref: RefObject<HTMLElement | null>, expanded: boolean, workAreaKey = ""): void {
  const lastReport = useRef<{ expanded: boolean; height: number; workAreaKey: string } | null>(null);

  useEffect(() => {
    const element = ref.current;
    if (!element) return;

    let timer: ReturnType<typeof setTimeout> | undefined;
    let pending: number | null = null;
    let retries = 0;
    let disposed = false;
    const report = (height: number | null) => {
      if (disposed) return;
      if (height !== pending) retries = 0;
      pending = height;
      if (height === null) {
        clearTimeout(timer);
        timer = undefined;
        return;
      }
      if (timer !== undefined) return;
      timer = setTimeout(() => {
        timer = undefined;
        if (pending === null || disposed) return;
        const previous = lastReport.current;
        if (
          previous?.height === pending &&
          previous.expanded === expanded &&
          previous.workAreaKey === workAreaKey
        )
          return;
        const measurement = { expanded, height: pending, workAreaKey };
        lastReport.current = measurement;
        void usePanelStore
          .getState()
          .setExpanded(expanded, pending)
          .then((succeeded) => {
            if (disposed || succeeded || lastReport.current !== measurement) return;
            lastReport.current = null;
            // A transient IPC failure must not permanently deduplicate a static
            // frame. Retry once, re-reading current content; never poll forever.
            if (pending === measurement.height && retries < 1) {
              retries += 1;
              report(measuredFrameHeight(element));
            }
          });
      }, 80);
    };

    const observer =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver((entries) => {
            const entry = entries.find((item) => item.target === element);
            if (entry) report(measuredFrameHeight(element, entry));
          });
    observer?.observe(element, { box: "border-box" });
    report(measuredFrameHeight(element));

    return () => {
      disposed = true;
      observer?.disconnect();
      clearTimeout(timer);
    };
  }, [ref, expanded, workAreaKey]);
}
