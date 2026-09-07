import { useEffect, useMemo, useRef } from "react";

import { debounce, type Debounced } from "@/lib/utils/debounce";

/**
 * Debounced callback that always sees the latest closure and flushes on
 * unmount (so pending saves are not lost when a pane closes).
 */
export function useDebouncedCallback<A extends unknown[]>(fn: (...args: A) => void, waitMs: number): Debounced<A> {
  const fnRef = useRef(fn);
  fnRef.current = fn;

  const debounced = useMemo(() => debounce<A>((...args) => fnRef.current(...args), waitMs), [waitMs]);

  useEffect(() => () => debounced.flush(), [debounced]);

  return debounced;
}
