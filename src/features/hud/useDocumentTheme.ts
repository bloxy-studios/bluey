import { useSyncExternalStore } from "react";

export type DocumentTheme = "dark" | "light";

function readTheme(): DocumentTheme {
  return document.documentElement.dataset.theme === "light" ? "light" : "dark";
}

function subscribe(onChange: () => void): () => void {
  const observer = new MutationObserver(onChange);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  return () => observer.disconnect();
}

/**
 * The applied theme (`data-theme` on the root, set by the bootstrap from the
 * appearance setting and the system scheme). Renderers that paint their own
 * colours — shiki, mermaid — follow it live, not only when first mounted (UX-005).
 */
export function useDocumentTheme(): DocumentTheme {
  return useSyncExternalStore(subscribe, readTheme, () => "dark");
}
