#!/bin/sh
# Build the host-native module in release and time its put calls.
#
# tools/putcost.lua reports microseconds per put call from a stock Lua 5.1,
# and a debug build would time the encoder's assertions rather than the
# crossing, so this builds release and looks under target/release. The rest
# follows tools/luatest.sh: the Windows skip, the Lua check, and the two
# module spellings.
#
# Run it on a quiet machine: `mise run bench-put`, or pass a call count as the
# one argument.
#
# POSIX sh only. Needs lua 5.1; mise.toml pins the version.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN* | Windows_NT)
        printf 'skipped: a Windows module resolves no Lua symbol at load.\n'
        printf 'Time the product DLL inside DCS instead; see ADR 0013.\n'
        exit 0
        ;;
esac

LUA=${LUA:-lua}
command -v "$LUA" >/dev/null 2>&1 || {
    printf 'no %s on PATH. mise.toml pins the version:\n\n' "$LUA" >&2
    printf '  mise install\n  mise exec -- sh tools/putcost.sh\n' >&2
    exit 2
}

case "$("$LUA" -v 2>&1)" in
    *"Lua 5.1"*) ;;
    *)
        printf '%s is not Lua 5.1: %s\n' "$LUA" "$("$LUA" -v 2>&1)" >&2
        printf 'The module binds the stock 5.1 API and DCS ships 5.1.\n' >&2
        exit 2
        ;;
esac

(cd "$ROOT" && cargo build --release -p lua-dcsbridge --no-default-features)

MODULE=
for candidate in \
    "$ROOT/target/release/liblua_dcsbridge.so" \
    "$ROOT/target/release/liblua_dcsbridge.dylib"
do
    [ -f "$candidate" ] && MODULE=$candidate
done

[ -n "$MODULE" ] || {
    printf 'no module under %s/target/release.\n' "$ROOT" >&2
    printf 'Expected liblua_dcsbridge.so or liblua_dcsbridge.dylib.\n' >&2
    exit 1
}

"$LUA" "$ROOT/tools/putcost.lua" "$MODULE" "${1:-}"
