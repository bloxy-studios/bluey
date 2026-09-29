import { AlertCircle } from "lucide-react";
import { Component, type ReactNode } from "react";

import { Button } from "./Button";

export interface ErrorBoundaryProps {
  children: ReactNode;
  /** Shown instead of the children once they threw; the window-level fallback by default. */
  fallback?: ReactNode;
  /** Remount the children when this changes (e.g. the item the boundary wraps). */
  resetKey?: unknown;
}

interface ErrorBoundaryState {
  failed: boolean;
  resetKey: unknown;
}

/**
 * Contains a render exception to the subtree that threw (UX-038): a window root
 * shows a compact "Reload" card instead of going blank, and one bad chat turn or
 * diagram no longer takes the whole HUD down. React reports the caught error to
 * the console itself; nothing here logs the content that failed to render.
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  override state: ErrorBoundaryState = { failed: false, resetKey: this.props.resetKey };

  static getDerivedStateFromError(): Partial<ErrorBoundaryState> {
    return { failed: true };
  }

  static getDerivedStateFromProps(
    props: ErrorBoundaryProps,
    state: ErrorBoundaryState,
  ): Partial<ErrorBoundaryState> | null {
    return Object.is(props.resetKey, state.resetKey) ? null : { failed: false, resetKey: props.resetKey };
  }

  override render(): ReactNode {
    if (!this.state.failed) return this.props.children;
    return this.props.fallback ?? <WindowErrorFallback />;
  }
}

/** The window-root fallback: say so, and offer the one fix that always works. */
export function WindowErrorFallback() {
  return (
    <div className="flex h-full items-start justify-center p-4">
      <div
        role="alert"
        className="flex w-full max-w-[420px] items-center gap-3 rounded-card border border-hud-border bg-bg-elevated px-4 py-3 text-fg shadow-lg shadow-black/25"
      >
        <AlertCircle className="size-4 shrink-0 text-danger" aria-hidden />
        <p className="m-0 min-w-0 flex-1 text-[13px]">Something went wrong.</p>
        <Button size="sm" variant="secondary" onClick={() => window.location.reload()}>
          Reload
        </Button>
      </div>
    </div>
  );
}
