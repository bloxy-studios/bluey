/**
 * Geometry is in CSS px / macOS logical points, never device pixels.
 * Keep these insets in sync with bluey-protocols/src/panel.rs. Appearance
 * width is the bordered surface; PanelState width/height describe its frame.
 */
export const HUD_FRAME_INSETS = { top: 24, right: 32, bottom: 40, left: 32 } as const;
export const HUD_MAX_SURFACE_HEIGHT = 620;

export function hudFrameWidth(surfaceWidth: number): number {
  return surfaceWidth + HUD_FRAME_INSETS.left + HUD_FRAME_INSETS.right;
}

/** Use a work-area limit, NOT the auto-sized window's current innerHeight. */
export function hudSurfaceMaxHeight(workAreaHeight: number): number {
  const available =
    Number.isFinite(workAreaHeight) && workAreaHeight > 0
      ? workAreaHeight - HUD_FRAME_INSETS.top - HUD_FRAME_INSETS.bottom
      : HUD_MAX_SURFACE_HEIGHT;
  return Math.max(1, Math.min(HUD_MAX_SURFACE_HEIGHT, available));
}

/**
 * Observe the outer frame, including padding and the surface's two borders.
 * contentRect excludes the observed element's padding/border. Old WebKit's
 * missing borderBoxSize falls back to the same border-box as the initial read.
 * The frame itself must not be transformed by the surface's entry animation.
 */
export function measuredFrameHeight(element: HTMLElement, entry?: ResizeObserverEntry): number | null {
  const borderHeight = entry?.borderBoxSize?.[0]?.blockSize;
  const height = borderHeight ?? element.getBoundingClientRect().height;
  return Number.isFinite(height) && height > 0 ? Math.ceil(height) : null;
}
