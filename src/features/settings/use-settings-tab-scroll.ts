import { useCallback, useLayoutEffect, useRef, useState } from "react";

/** Keep scrolling local to the toolbar — scrollIntoView can also move the page. */
export function useSettingsTabScroll(selectedTab: string) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState({ earlier: false, later: false });

  const measure = useCallback(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const max = Math.max(0, viewport.scrollWidth - viewport.clientWidth);
    // A pixel of tolerance avoids flickering controls at fractional zoom levels.
    // WebKit rubber-banding can report offsets outside the scrollable range.
    const offset = Math.max(0, Math.min(max, viewport.scrollLeft));
    const earlier = offset > 1;
    const later = offset < max - 1;
    setEdges((previous) =>
      previous.earlier === earlier && previous.later === later ? previous : { earlier, later },
    );
  }, []);

  const reveal = useCallback(
    (item: HTMLElement | null) => {
      const viewport = viewportRef.current;
      if (!viewport || !item) return;
      const bounds = viewport.getBoundingClientRect();
      const target = item.getBoundingClientRect();
      const gutter = 4; // Leave room for the keyboard focus ring.
      const delta = target.left < bounds.left + gutter
        ? target.left - bounds.left - gutter
        : Math.max(0, target.right - bounds.right + gutter);
      if (delta !== 0) {
        const max = Math.max(0, viewport.scrollWidth - viewport.clientWidth);
        viewport.scrollLeft = Math.max(0, Math.min(max, viewport.scrollLeft + delta));
      }
      measure();
    },
    [measure],
  );

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    const list = listRef.current;
    if (!viewport || !list) return;
    const onResize = () => {
      const focused = document.activeElement;
      reveal(
        focused instanceof HTMLElement && list.contains(focused)
          ? focused
          : list.querySelector<HTMLElement>('[aria-selected="true"]'),
      );
    };
    const observer = new ResizeObserver(onResize);
    observer.observe(viewport);
    observer.observe(list);
    viewport.addEventListener("scroll", measure, { passive: true });
    onResize();
    return () => {
      observer.disconnect();
      viewport.removeEventListener("scroll", measure);
    };
  }, [measure, reveal]);

  useLayoutEffect(() => {
    reveal(listRef.current?.querySelector<HTMLElement>('[aria-selected="true"]') ?? null);
  }, [selectedTab, reveal]);

  const scroll = (direction: -1 | 1) => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const max = Math.max(0, viewport.scrollWidth - viewport.clientWidth);
    viewport.scrollLeft = Math.max(
      0,
      Math.min(max, viewport.scrollLeft + direction * viewport.clientWidth * 0.75),
    );
    measure();
  };

  return { viewportRef, listRef, edges, reveal, scroll };
}
