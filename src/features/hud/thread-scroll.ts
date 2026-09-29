/** Breathing room kept above a turn pinned to the top of the thread viewport. */
export const TURN_TOP_MARGIN_PX = 12;

export interface ScrollView {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}

/**
 * Where following a streaming turn should scroll (UX-027): the bottom, but
 * never past the newest turn's first line — a long answer's opening ("say
 * this") stays in view and the ↓ button takes over. A reader already below
 * that line keeps following the bottom.
 */
export function followScrollTop(view: ScrollView, turnTop: number): number {
  const bottom = Math.max(0, view.scrollHeight - view.clientHeight);
  const pin = Math.max(0, turnTop - TURN_TOP_MARGIN_PX);
  return view.scrollTop > pin ? bottom : Math.min(bottom, pin);
}

/** The element's top in the scroll container's content coordinates. */
export function offsetInScroller(scroller: HTMLElement, element: HTMLElement): number {
  return element.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop;
}
