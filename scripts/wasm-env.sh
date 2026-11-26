# Source this before building anything for wasm32-unknown-unknown:
#
#   source scripts/wasm-env.sh && cargo build -p dv-wasm --target wasm32-unknown-unknown
#
# The .pbix writer compiles C code (XPress9 compression, in the ripbi-xpress9 crate) that
# needs a few libc functions, which WebAssembly does not have. The zstd-sys crate, already in
# the dependency tree, ships a small libc shim for exactly this (malloc, free, memcpy, memset
# that call into Rust), so this points the C compiler at its headers. XPress9 also calls
# __rdtsc() once, only to seed a statistics signature; a constant does as well.
#
# SQLite needs nothing here: rusqlite uses the sqlite-wasm-rs crate, which brings its own.

# The first interpreter that actually runs (on Windows `python3` can be a Store stub).
_py=""
for _candidate in python3 python; do
  if "$_candidate" -c "" >/dev/null 2>&1; then
    _py="$_candidate"
    break
  fi
done
if [ -z "$_py" ]; then
  echo "wasm-env.sh: Python is needed to read cargo metadata" >&2
  return 1
fi
_zstd_manifest="$(cargo metadata --format-version 1 --filter-platform wasm32-unknown-unknown |
  "$_py" -c 'import json, sys
for p in json.load(sys.stdin)["packages"]:
    if p["name"] == "zstd-sys":
        print(p["manifest_path"])
        break')"
if [ -z "$_zstd_manifest" ]; then
  echo "wasm-env.sh: zstd-sys is not in the dependency tree; cannot find the libc shim" >&2
  return 1
fi
_shim="$(dirname "$_zstd_manifest")/wasm-shim"
# Windows shells give a path like /c/Users/...; clang needs C:/Users/...
if command -v cygpath >/dev/null 2>&1; then
  _shim="$(cygpath -m "$_shim")"
fi
export CFLAGS_wasm32_unknown_unknown="-I$_shim -D__rdtsc()=0x9E3779B97F4A7C15ULL"
unset _py _candidate _zstd_manifest _shim
