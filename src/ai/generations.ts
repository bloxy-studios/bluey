/**
 * GenerationGate: stale-response protection (spec §61–62).
 *
 * Each scope (e.g. the ask window, the prepare pipeline) has a monotonic
 * generation counter. A pipeline records the generation it started with and
 * checks `isStale` before publishing anything; a newer generation in the same
 * scope means the old result must be discarded, never rendered.
 */

export class GenerationGate {
  private readonly counters = new Map<string, number>();
  private readonly inflight = new Map<string, string>();

  /** Bump and return the next generation for a scope. */
  next(scope: string): number {
    const value = (this.counters.get(scope) ?? 0) + 1;
    this.counters.set(scope, value);
    return value;
  }

  current(scope: string): number {
    return this.counters.get(scope) ?? 0;
  }

  /** True when a newer generation has started in this scope. */
  isStale(scope: string, generation: number): boolean {
    return generation < this.current(scope);
  }

  /** Invalidate everything in a scope (e.g. cancelAll). */
  invalidate(scope: string): number {
    return this.next(scope);
  }

  invalidateAll(): void {
    for (const scope of this.counters.keys()) this.next(scope);
  }

  /** Track the request id currently in flight for a scope. */
  setInflight(scope: string, requestId: string): void {
    this.inflight.set(scope, requestId);
  }

  /** Take (and clear) the previous in-flight request id for a scope, if any. */
  takeInflight(scope: string): string | undefined {
    const requestId = this.inflight.get(scope);
    this.inflight.delete(scope);
    return requestId;
  }

  clearInflight(scope: string, requestId: string): void {
    if (this.inflight.get(scope) === requestId) this.inflight.delete(scope);
  }

  inflightIds(): string[] {
    return Array.from(this.inflight.values());
  }
}
