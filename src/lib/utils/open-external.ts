import { showErrorToast } from "@/components/ui/toast-store";
import { hasTauriRuntime } from "@/lib/tauri/transport";
import type { BlueyError } from "@/lib/types";

/** Schemes the opener capabilities allow (src-tauri/capabilities/*.json). */
const OPENABLE_SCHEMES = new Set(["http:", "https:", "mailto:"]);

function linkError(code: "not_supported.link_scheme" | "internal.open_link", message: string): BlueyError {
  const kind = code === "internal.open_link" ? "internal" : "not_supported";
  return { kind, code, message, recoverable: false };
}

/**
 * Open a web or email link in the user's default app (opener plugin inside
 * Tauri). Other schemes (javascript:, file:, app-specific) are refused, and
 * any failure is shown as a toast instead of disappearing (UX-006) — callers
 * can fire and forget.
 */
export async function openExternal(url: string): Promise<void> {
  let scheme: string;
  try {
    scheme = new URL(url).protocol;
  } catch {
    scheme = "";
  }
  if (!OPENABLE_SCHEMES.has(scheme)) {
    showErrorToast(linkError("not_supported.link_scheme", `refused to open a ${scheme || "malformed"} link`));
    return;
  }
  try {
    if (hasTauriRuntime()) {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(url);
      return;
    }
    window.open(url, "_blank", "noopener,noreferrer");
  } catch (error) {
    showErrorToast(linkError("internal.open_link", error instanceof Error ? error.message : String(error)));
  }
}
