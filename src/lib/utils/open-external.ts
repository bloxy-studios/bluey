import { hasTauriRuntime } from "@/lib/tauri/transport";

/** Open a URL in the user's default browser (opener plugin inside Tauri). */
export async function openExternal(url: string): Promise<void> {
  if (hasTauriRuntime()) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}
