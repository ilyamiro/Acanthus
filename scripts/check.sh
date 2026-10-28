#!/usr/bin/env bash
set -u
cd "$(dirname "$0")/.."
mode=full
pkgs=()
for arg in "$@"; do
  case "$arg" in
    --full) mode=full ;;
    *) mode=quick; pkgs+=("-p" "$arg") ;;
  esac
done
scope=(--workspace)
[ "${#pkgs[@]}" -gt 0 ] && scope=("${pkgs[@]}")
fail=0
step() {
  local name=$1; shift
  local out
  if out=$(nix-shell --run "$*" 2>&1); then
    echo "ok    $name"
  else
    echo "FAIL  $name"
    echo "$out" | grep -E '^(error|warning)|panicked|FAILED|failed|^\s+-->' | head -40
    fail=1
  fi
}
step fmt "cargo fmt --all"
step layers "scripts/check-layers.sh"
step build "cargo build ${scope[*]} --all-targets"
if [ "$mode" = full ]; then
  step deny "cargo deny check licenses bans sources"
  step features "cargo check -p acanthus-services --no-default-features"
  step doc "RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --workspace"
fi
step clippy "cargo clippy ${scope[*]} --all-targets -- -D warnings"
test_log=$(nix-shell --run "cargo nextest run ${scope[*]} --no-fail-fast && cargo test ${scope[*]} --doc" 2>&1)
status=$?
total=$(echo "$test_log" | grep -E 'Summary' | sed -E 's/.* ([0-9]+) tests run.*/\1/' | head -1)
if [ $status -eq 0 ]; then
  echo "ok    test (${total:-0} passed)"
else
  echo "FAIL  test"
  echo "$test_log" | grep -E 'FAIL |^test .* FAILED|panicked|^error' -A3 | head -40
  fail=1
fi
hits=$(grep -rn '//' crates --include='*.rs' --include='*.wgsl' | grep -v '[a-z]://' | grep -vE '^crates/[^:]+:[0-9]+:[[:space:]]*//[/!]([^/]|$)' | head -5)
if [ -z "$hits" ]; then echo "ok    no-comments"; else echo "FAIL  no-comments"; echo "$hits"; fail=1; fi
[ "$mode" = quick ] && echo "note: quick run on ${pkgs[*]}; run scripts/check.sh before the final commit"
exit $fail
