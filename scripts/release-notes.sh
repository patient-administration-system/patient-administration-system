#!/usr/bin/env bash
# Print the CHANGELOG entry for a version, and fail if there is none or the tag does not match the
# server crate's version. Used by the release workflow.
#
#   scripts/release-notes.sh 0.45.0
set -euo pipefail
cd "$(dirname "$0")/.."
version="${1:?usage: release-notes.sh <version, without the v>}"
crate="$(sed -n 's/^version *= *"\(.*\)"/\1/p' patient-administration-system-with-rust/Cargo.toml | head -1)"
[[ "$crate" == "$version" ]] || { echo "tag $version != server crate version $crate" >&2; exit 1; }
notes="$(awk -v v="$version" '
  $0 ~ "^## \\[" v "\\]" { on=1; next }
  on && /^## / { exit }
  on { print }' patient-administration-system-with-rust/CHANGELOG.md)"
[[ -n "${notes//[[:space:]]/}" ]] || { echo "no CHANGELOG entry for $version" >&2; exit 1; }
printf '%s\n' "$notes"
