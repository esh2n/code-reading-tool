#!/usr/bin/env bash
# Verifies the dependency rule from docs/architecture.md mechanically:
# inner crates must not depend on outer ones. Cargo already refuses an
# undeclared import; this checks the declared graph itself.
set -euo pipefail

cd "$(dirname "$0")/.."

fail=0

deps_of() {
  cargo tree -p "$1" -e normal --depth 1 --prefix none 2>/dev/null | tail -n +2 | awk '{print $1}' | sort -u
}

check() {
  local crate=$1; shift
  local allowed=" $* "
  while read -r dep; do
    [ -z "$dep" ] && continue
    case "$allowed" in
      *" $dep "*) ;;
      *) echo "layer violation: $crate depends on $dep" >&2; fail=1 ;;
    esac
  done < <(deps_of "$crate")
}

# crate            allowed direct dependencies
check crt-domain
check crt-app      crt-domain
check crt-wire     crt-domain crt-app serde
check crt-treesitter crt-domain crt-app tree-sitter tree-sitter-tags \
  tree-sitter-rust tree-sitter-go tree-sitter-python tree-sitter-javascript

if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "layers ok"
