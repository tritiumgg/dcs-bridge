#!/bin/sh
# Build the host-native module and open it with a stock Lua 5.1.
#
# SPEC 17's Host column marks about half the test rows *Any (native module)*:
# runnable against a host-native build of the broker loaded by a stock Lua 5.1,
# with no DCS present. This is what carries them. Task 2.1 lands the load
# itself; each row arrives with the behaviour it describes.
#
# --no-default-features turns the dcs-lua feature off, which is what the
# host-native build is. Left on, the module would try to link DCS's lua.dll,
# and a machine with no DCS has none to load. ADR 0002, ADR 0006.
#
# Linux and macOS only. A Windows module resolves no symbol at load, so a
# host-native build there would need a fetched lua51.dll and a second .def for
# a configuration that never ships. The Windows runner builds and tests the
# broker, and the cross-build proves the product DLL imports lua.dll.
#
# POSIX sh only. Needs lua 5.1; mise.toml pins the version.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN* | Windows_NT)
        printf 'skipped: a Windows module resolves no Lua symbol at load.\n'
        printf 'The product path is cargo xwin build; see mise run windows.\n'
        exit 0
        ;;
esac

LUA=${LUA:-lua}
command -v "$LUA" >/dev/null 2>&1 || {
    printf 'no %s on PATH. mise.toml pins the version:\n\n' "$LUA" >&2
    printf '  mise install\n  mise exec -- sh tools/luatest.sh\n' >&2
    exit 2
}

case "$("$LUA" -v 2>&1)" in
    *"Lua 5.1"*) ;;
    *)
        printf '%s is not Lua 5.1: %s\n' "$LUA" "$("$LUA" -v 2>&1)" >&2
        printf 'SPEC 5.1.1 binds the stock 5.1 API and DCS ships 5.1.\n' >&2
        exit 2
        ;;
esac

(cd "$ROOT" && cargo build -p lua-dcsbridge --no-default-features)

# One of the two, depending on the host. Nothing globs a third.
MODULE=
for candidate in \
    "$ROOT/target/debug/liblua_dcsbridge.so" \
    "$ROOT/target/debug/liblua_dcsbridge.dylib"
do
    [ -f "$candidate" ] && MODULE=$candidate
done

[ -n "$MODULE" ] || {
    printf 'no module under %s/target/debug.\n' "$ROOT" >&2
    printf 'Expected liblua_dcsbridge.so or liblua_dcsbridge.dylib.\n' >&2
    exit 1
}

# The version the module reports has to match the one the workspace declares,
# or the harness proves only that some module opened.
VERSION=$(awk '
    /^\[workspace\.package\]/ { in_section = 1; next }
    /^\[/                     { in_section = 0 }
    in_section && /^version/  { gsub(/[^0-9A-Za-z.+-]/, "", $3); print $3; exit }
' "$ROOT/Cargo.toml")

[ -n "$VERSION" ] || {
    printf 'no [workspace.package] version in %s/Cargo.toml\n' "$ROOT" >&2
    exit 1
}

"$LUA" "$ROOT/tests/lua/load.lua" "$MODULE" "$VERSION"
