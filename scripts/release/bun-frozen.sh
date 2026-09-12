#!/usr/bin/env bash
# Release-only PATH shim. The unchanged sidecar helper invokes its own bun install.
# Force that nested install to honor its checked-in lock too, without changing dev behavior.
set -euo pipefail
: "${BLUEY_RELEASE_BUN:?original pinned Bun executable is required}"
case "${1:-}" in
  install|i)
    shift
    frozen=false
    for arg in "$@"; do
      case "$arg" in
        --frozen-lockfile) frozen=true ;;
        --frozen-lockfile=*|--no-frozen-lockfile*)
          printf '%s\n' 'Release installs cannot disable frozen lockfiles.' >&2
          exit 1
          ;;
      esac
    done
    if [[ "$frozen" == true ]]; then
      exec "$BLUEY_RELEASE_BUN" install "$@"
    fi
    exec "$BLUEY_RELEASE_BUN" install --frozen-lockfile "$@"
    ;;
  *) exec "$BLUEY_RELEASE_BUN" "$@" ;;
esac
