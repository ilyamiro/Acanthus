#!/usr/bin/env bash
set -u
cd "$(dirname "$0")/.."
fail=0
report() { echo "layers: $*"; fail=1; }

meta=$(cargo metadata --format-version 1 --no-deps --offline 2>/dev/null || cargo metadata --format-version 1 --no-deps)

while IFS=$'\t' read -r pkg manifest; do
  case "$manifest" in
    "$PWD"/*) ;;
    *) report "$pkg manifest $manifest is outside the repository" ;;
  esac
done < <(echo "$meta" | jq -r '.packages[] | [.name, .manifest_path] | @tsv')

while IFS=$'\t' read -r pkg dep path; do
  [ -z "$path" ] && continue
  case "$(realpath -m "$path")" in
    "$PWD"/*) ;;
    *) report "$pkg depends on $dep through $path outside the repository" ;;
  esac
done < <(echo "$meta" | jq -r '.packages[] | .name as $p | .dependencies[] | select(.path != null) | [$p, .name, .path] | @tsv')

while IFS=$'\t' read -r pkg license; do
  case "$pkg:$license" in
    acanthus-color:"AGPL-3.0-or-later AND (MIT OR Apache-2.0)") ;;
    acanthus-color:*) report "acanthus-color license is '$license', expected AGPL-3.0-or-later AND (MIT OR Apache-2.0)" ;;
    *:AGPL-3.0-or-later) ;;
    *) report "$pkg license is '$license', expected AGPL-3.0-or-later" ;;
  esac
done < <(echo "$meta" | jq -r '.packages[] | [.name, (.license // "none")] | @tsv')

while IFS=: read -r file line text; do
  target=$(echo "$text" | sed -E 's/.*include_(str|bytes)!\("([^"]+)"\).*/\2/')
  crate_dir=$(echo "$file" | sed -E 's#^((crates|examples)/[^/]+|xtask)/.*#\1#')
  resolved=$(realpath -m "$(dirname "$file")/$target")
  case "$resolved" in
    "$PWD/$crate_dir"/*) ;;
    *) report "$file:$line includes $target outside $crate_dir" ;;
  esac
done < <(grep -rnE 'include_(str|bytes)!\("' crates examples xtask --include='*.rs' --exclude-dir=target)

if hits=$(grep -rnE 'serpantinum|Serpantinum|QS_' crates examples xtask assets docs deny.toml shell.nix Cargo.toml -I --exclude-dir=target 2>/dev/null); then
  echo "$hits" | while IFS= read -r line; do echo "layers: application name: $line"; done
  fail=1
fi

[ $fail -eq 0 ] && echo "layers: ok"
exit $fail
