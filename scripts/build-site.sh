#!/usr/bin/env bash
# Builds the static web site into ./site: the landing page, the WebAssembly report checker
# and HTML renderings of the example reports.
#
# Needs: the wasm32-unknown-unknown Rust target, wasm-bindgen-cli matching the wasm-bindgen
# version in Cargo.lock, and a built davilize CLI (pass its path as $DAVILIZE, default
# target/release/davilize).
set -euo pipefail
cd "$(dirname "$0")/.."

DAVILIZE="${DAVILIZE:-target/release/davilize}"
SITE=site
mkdir -p "$SITE"
find "$SITE" -mindepth 1 -delete
mkdir -p "$SITE/pkg" "$SITE/samples"

cargo build -p dv-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target web --no-typescript --out-dir "$SITE/pkg" \
  target/wasm32-unknown-unknown/release/dv_wasm.wasm
cp web/index.html "$SITE/index.html"
touch "$SITE/.nojekyll"

entries=()
for report in sample sample_dashboard sample_joined sample_params sample_scheduled; do
  "$DAVILIZE" refresh "examples/$report.dvz" --out "$SITE/samples/$report" --format html >/dev/null
  entries+=("{\"name\":\"$report\",\"href\":\"samples/$report/$report.html\"}")
done
printf '[%s]\n' "$(IFS=,; echo "${entries[*]}")" > "$SITE/samples/index.json"
# Reports and the data files they read, for the in-browser runner.
mkdir -p "$SITE/examples"
runnable=()
for report in sample sample_dashboard sample_params; do
  cp "examples/$report.dvz" "$SITE/examples/"
  files="\"$report.dvz\""
  for data in $(grep -o '"path": *"[^"]*"' "examples/$report.dvz" | sed 's/.*: *"\(.*\)"/\1/' | sort -u); do
    cp "examples/$data" "$SITE/examples/"
    files="$files,\"$data\""
  done
  runnable+=("{\"name\":\"$report\",\"files\":[$files]}")
done
printf '[%s]\n' "$(IFS=,; echo "${runnable[*]}")" > "$SITE/examples/index.json"

echo "site built in $SITE"
