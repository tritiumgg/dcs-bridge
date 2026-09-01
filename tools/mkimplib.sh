#!/bin/sh
# Generate a Windows import library from a .def file.
#
# The broker links against DCS's Lua by name. The import library is derived
# from vendor/lua/lua.def, so no DCS install is needed at build time and the
# set of Lua symbols the broker may depend on is pinned to that file.
#
# Three tools can do this and the first one found wins:
#
#   llvm-dlltool   a full LLVM install (apt install llvm, brew install llvm)
#   llvm-lib       the same install, MSVC-compatible front end
#   lib.exe        MSVC, on a Windows host with the Build Tools
#
# POSIX sh only. Run it by hand to check the .def is usable on this machine.
# The broker's build.rs runs the same probe in Rust, so a Windows host with no
# sh still builds.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
DEF=${1:-$ROOT/vendor/lua/lua.def}
OUT=${2:-$ROOT/target/lua.lib}

[ -f "$DEF" ] || { printf 'no .def at %s\n' "$DEF" >&2; exit 2; }
mkdir -p "$(dirname "$OUT")"

find_tool() {
    for t in "$@"; do
        if command -v "$t" >/dev/null 2>&1; then printf '%s' "$t"; return 0; fi
    done
    # A full LLVM install often lands versioned and off PATH.
    for d in /usr/lib/llvm-*/bin /opt/homebrew/opt/llvm/bin /usr/local/opt/llvm/bin; do
        for t in "$@"; do
            if [ -x "$d/$t" ]; then printf '%s' "$d/$t"; return 0; fi
        done
    done
    return 1
}

if tool=$(find_tool llvm-dlltool); then
    "$tool" -m i386:x86-64 -d "$DEF" -l "$OUT"
    used="$tool -m i386:x86-64"
elif tool=$(find_tool llvm-lib lib.exe lib); then
    "$tool" "/def:$DEF" "/out:$OUT" /machine:x64
    used="$tool /machine:x64"
else
    cat >&2 <<'EOF'
No import-library tool found. Install one:

  Debian or Ubuntu   apt-get install llvm
  macOS              brew install llvm
  Windows            the MSVC Build Tools, which provide lib.exe

llvm-dlltool and llvm-lib both ship with a full LLVM install. A rustup
llvm-tools component is not enough on its own.
EOF
    exit 2
fi

printf 'wrote %s (%s bytes) using %s\n' "${OUT#"$ROOT"/}" "$(wc -c < "$OUT" | tr -d ' ')" "$used"

# The library is useless if it names the wrong DLL, so say which one it names.
if strings "$OUT" 2>/dev/null | grep -q '^lua\.dll$'; then
    printf 'links against lua.dll, as SPEC 5.1.1 requires\n'
else
    printf 'WARNING: the output does not name lua.dll. Check the LIBRARY line in %s\n' "$DEF" >&2
    exit 1
fi
