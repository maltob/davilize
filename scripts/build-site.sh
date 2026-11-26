#!/usr/bin/env bash
# Builds the static web site into ./site: the Davilize app running in the browser, plus HTML
# renderings of the example reports under samples/.
#
# The app is the same React front end as the desktop app. In the browser its queries run in
# DuckDB-Wasm, and the rest of the engine (SQL compilation, joins, calculated-field checks,
# chart specs, HTML and XLSX writers) is the Rust `dv-wasm` crate compiled to WebAssembly.
#
# Needs: the wasm32-unknown-unknown Rust target, wasm-bindgen-cli matching the wasm-bindgen
# version in Cargo.lock, Node with npm, and a built davilize CLI (pass its path as
# $DAVILIZE, default target/release/davilize).
set -euo pipefail
cd "$(dirname "$0")/.."

DAVILIZE="${DAVILIZE:-target/release/davilize}"
SITE=site

# 1. The Rust side, as WebAssembly, served next to the app.
# shellcheck source=scripts/wasm-env.sh
source scripts/wasm-env.sh
cargo build -p dv-wasm --target wasm32-unknown-unknown --profile wasm
mkdir -p app/public/pkg app/public/examples
wasm-bindgen --target web --no-typescript --out-dir app/public/pkg \
  target/wasm32-unknown-unknown/wasm/dv_wasm.wasm

# 2. Example reports and their data, for "Try the sample report".
cp examples/*.dvz examples/*.csv app/public/examples/
# The database sample reads a SQLite file named after its connection (sample_db.sqlite).
_py=""
for _candidate in python3 python; do
  if "$_candidate" -c "" >/dev/null 2>&1; then _py="$_candidate"; break; fi
done
rm -f app/public/examples/sample_db.sqlite
"$_py" - <<'PY'
import sqlite3
db = sqlite3.connect("app/public/examples/sample_db.sqlite")
db.executescript(open("examples/sample_sqlite.sql", encoding="utf-8").read())
db.commit()
db.close()
PY

# 3. The app.
(cd app && npm ci && npm run build:web)

mkdir -p "$SITE"
find "$SITE" -mindepth 1 -delete
cp -r app/dist/. "$SITE/"
touch "$SITE/.nojekyll"

# 4. HTML renderings of the examples that need no database.
mkdir -p "$SITE/samples"
items=""
for report in sample sample_dashboard sample_joined sample_params sample_scheduled; do
  "$DAVILIZE" refresh "examples/$report.dvz" --out "$SITE/samples/$report" --format html >/dev/null
  items="$items<li><a href=\"$report/$report.html\">$report</a></li>"
done
cat > "$SITE/samples/index.html" <<HTML
<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Davilize samples</title></head>
<body style="font:16px/1.5 system-ui,sans-serif;max-width:40rem;margin:2rem auto;padding:0 1rem">
<h1>Sample reports</h1>
<p>Pages written by <code>davilize refresh</code> from the example reports. <a href="../">Open the app</a>.</p>
<ul>$items</ul>
</body></html>
HTML

echo "site built in $SITE"
