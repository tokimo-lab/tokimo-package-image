#!/usr/bin/env bash
# Fix hardcoded Homebrew absolute paths in tokimo-lib macOS dylibs.
#
# The tokimo-lib release artifact ships dylibs that reference Homebrew Cellar
# paths (e.g. /opt/homebrew/Cellar/glib/2.88.1/lib/libglib-2.0.0.dylib)
# instead of @rpath.  CI runners don't have these paths, causing SIGSEGV.
#
# This script rewrites the references to @rpath and re-signs the dylibs.
#
# Usage: bash scripts/fixup-tokimo-lib-dylibs.sh <lib-dir>

set -euo pipefail

LIB_DIR="${1:?Usage: $0 <lib-dir>}"

if [[ ! -d "$LIB_DIR" ]]; then
  echo "Error: $LIB_DIR is not a directory"
  exit 1
fi

# ── Rewrite Homebrew absolute paths to @rpath ────────────────────────────────
# Each line: old_path -> new_path

CHANGES=(
  "/opt/homebrew/Cellar/glib/2.88.1/lib/libglib-2.0.0.dylib|@rpath/libglib-2.0.dylib"
  "/opt/homebrew/Cellar/glib/2.88.1/lib/libgobject-2.0.0.dylib|@rpath/libgobject-2.0.dylib"
  "/opt/homebrew/opt/glib/lib/libglib-2.0.0.dylib|@rpath/libglib-2.0.dylib"
  "/opt/homebrew/opt/glib/lib/libgio-2.0.0.dylib|@rpath/libgio-2.0.dylib"
  "/opt/homebrew/opt/glib/lib/libgobject-2.0.0.dylib|@rpath/libgobject-2.0.dylib"
  "/opt/homebrew/opt/zstd/lib/libzstd.1.dylib|@rpath/libzstd.1.dylib"
)

changed=0
for dylib in "$LIB_DIR"/*.dylib; do
  [[ -L "$dylib" ]] && continue
  [[ -f "$dylib" ]] || continue

  for entry in "${CHANGES[@]}"; do
    old="${entry%%|*}"
    new="${entry#*|}"
    if otool -L "$dylib" 2>/dev/null | grep -qF "$old"; then
      install_name_tool -change "$old" "$new" "$dylib" 2>/dev/null || true
      changed=$((changed + 1))
    fi
  done
done

# ── Bundle libzstd if missing (required by libtiff) ──────────────────────────

if [[ ! -f "$LIB_DIR/libzstd.1.dylib" ]]; then
  for candidate in \
    /opt/homebrew/opt/zstd/lib/libzstd.1.dylib \
    /opt/homebrew/lib/libzstd.1.dylib \
    /usr/local/lib/libzstd.1.dylib; do
    if [[ -f "$candidate" ]]; then
      cp "$candidate" "$LIB_DIR/libzstd.1.dylib"
      echo "Copied libzstd from $candidate"
      break
    fi
  done
fi

# ── Re-sign all modified dylibs ──────────────────────────────────────────────

if [[ $changed -gt 0 ]]; then
  for dylib in "$LIB_DIR"/*.dylib; do
    [[ -L "$dylib" ]] && continue
    codesign --force --sign - "$dylib" 2>/dev/null || true
  done
  echo "Fixed $changed Homebrew path references in $LIB_DIR"
else
  echo "No Homebrew path references found in $LIB_DIR"
fi
