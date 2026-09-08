import { useCallback } from "react";

import { presentError, type ErrorPresenterOptions, type PresentedError } from "@/lib/errors/present";
import type { BlueyError } from "@/lib/types";

export type { ErrorPresenterOptions, PresentedError } from "@/lib/errors/present";

/** React wrapper over `presentError`: a stable presenter bound to the given options. */
export function useErrorPresenter(options: ErrorPresenterOptions = {}) {
  const { onRetry, onOpenSettingsTab } = options;

  return useCallback(
    (error: BlueyError): PresentedError => presentError(error, { onRetry, onOpenSettingsTab }),
    [onRetry, onOpenSettingsTab],
  );
}
