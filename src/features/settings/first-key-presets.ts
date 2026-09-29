import { showErrorToast, useToastStore } from "@/components/ui/toast-store";
import { presetForKind } from "@/lib/ai/provider-presets";
import { bluey } from "@/lib/tauri/api";
import { toBlueyError, type AIProviderConfig, type ModelRoleAssignments } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";

const UNDO_TOAST_MS = 8000;

/**
 * A provider's first key puts it to work (FEATURE-004): its recommended models fill the roles
 * nothing serves yet (`overwrite: false`), and it becomes the default provider when there is
 * none. Assigned roles are never touched. A toast says what changed, with Undo.
 */
export async function adoptFirstKey(provider: AIProviderConfig): Promise<void> {
  const store = useSettingsStore.getState();
  const before = store.settings?.ai;
  if (!before || !presetForKind(provider.kind)) return;
  try {
    const next = await bluey.ai.applyProviderPresets({ providerId: provider.id, overwrite: false });
    store.applyRemote(next);
    const filled = filledRoles(before.models, next.ai.models);
    const becomesDefault = !before.bootstrapProvider && !before.models.default;
    if (becomesDefault) await store.update({ ai: { bootstrapProvider: provider.id } });
    if (filled === 0 && !becomesDefault) return;
    useToastStore.getState().push({
      message: `Roles now use ${provider.name}'s recommended models`,
      durationMs: UNDO_TOAST_MS,
      action: {
        label: "Undo",
        run: () => {
          void useSettingsStore.getState().update({
            ai: { models: before.models, bootstrapProvider: before.bootstrapProvider ?? null },
          });
        },
      },
    });
  } catch (error) {
    showErrorToast(toBlueyError(error, "configuration"));
  }
}

/** How many roles went from unassigned to assigned. */
function filledRoles(before: ModelRoleAssignments, after: ModelRoleAssignments): number {
  return (Object.keys(after) as (keyof ModelRoleAssignments)[]).filter((role) => !before[role] && after[role])
    .length;
}
