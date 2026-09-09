/** WKWebView can report isComposing=false on the IME's confirming keydown. */
export function isComposingKey(event: Pick<KeyboardEvent, "isComposing" | "keyCode">): boolean {
  return event.isComposing || event.keyCode === 229;
}

/** Holding Enter on a button must not synthesize repeated asks/new-chat clicks. */
export function preventRepeatedActivation(
  event: Pick<KeyboardEvent, "key" | "repeat" | "preventDefault">,
): void {
  if (event.repeat && (event.key === "Enter" || event.key === " ")) event.preventDefault();
}

/**
 * Only active overlays, never tooltips or closed/force-mounted content. The HUD
 * itself has role=dialog, so dialogs must also have overlay state/modal semantics.
 */
export function hasActiveHudOverlay(): boolean {
  const overlays = document.querySelectorAll(
    '[role="menu"], [role="listbox"], [role="dialog"][data-state], ' +
      '[role="alertdialog"][data-state], [aria-modal="true"], dialog[open]',
  );
  return Array.from(overlays).some(
    (overlay) =>
      overlay.getAttribute("data-state") !== "closed" &&
      !overlay.closest('[hidden], [aria-hidden="true"], [inert]'),
  );
}
