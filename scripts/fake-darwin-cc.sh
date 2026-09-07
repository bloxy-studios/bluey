#!/usr/bin/env bash
# Stub C/ObjC compiler for `cargo check --target aarch64-apple-darwin` on a Linux host.
# `cargo check` never links, so build scripts that compile C/ObjC sources via the
# `cc` crate only need *an* object file to exist. This wrapper:
#   * answers cc-rs's compiler-family probe (`-E detect_compiler_family.c`) as clang,
#   * rejects the MSVC probe (`-?`),
#   * emits an empty host object at the requested `-o`/`-Fo` path for `-c` compiles.
set -u
out=""
preprocess=0
for ((i=1; i<=$#; i++)); do
  arg="${!i}"
  case "$arg" in
    -o) j=$((i+1)); out="${!j}";;
    -Fo*) out="${arg#-Fo}";;
    -E) preprocess=1;;
    -\?) exit 1;;
    --version|-v|-dumpversion|-dumpmachine|-print-prog-name=*|-print-search-dirs)
      echo "Apple clang version 17.0.0 (stub)"; echo "Target: arm64-apple-darwin"; exit 0;;
  esac
done
if [[ $preprocess -eq 1 ]]; then
  # cc-rs greps this output for the family marker.
  echo "clang"
  exit 0
fi
if [[ -n "$out" ]]; then
  exec gcc -c -x c /dev/null -o "$out"
fi
exit 0
