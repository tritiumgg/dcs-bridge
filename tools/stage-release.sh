#!/bin/sh
# Stage the release artifacts from a Windows build directory.
#
# The CI cross-build and the release workflow both run this, so a pull request
# exercises the same staging a tag will run. It produces, under the output
# directory:
#
#   lua-dcsbridge.dll             the broker
#   dcsb.exe                      the CLI
#   write-directory-<v>.zip       the tree that extracts over the write directory
#   SHA256SUMS                    over all three
#
# Only those two binaries ship. protoc-gen-dcsbridge-lua is a build-time protoc
# plugin, so naming the artifacts here keeps it out of a release.
#
# The zip also carries schema.pb, which tools/mkschema.sh compiles. It is
# platform-independent and comes from the .proto tree rather than from the
# Windows build, so it has a path of its own rather than a place under BUILD.
#
# Usage: sh tools/stage-release.sh <build-dir> [version] [out-dir] [schema]
#
# POSIX sh only, and no required tool beyond zip and one of sha256sum or
# shasum. llvm-readobj is used for the export check when it is installed.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BUILD=${1:-$ROOT/target/x86_64-pc-windows-msvc/release}
VERSION=${2:-}
OUT=${3:-$ROOT/dist}
SCHEMA=${4:-$ROOT/target/schema.pb}

[ -d "$BUILD" ] || { printf 'no build directory at %s\n' "$BUILD" >&2; exit 2; }
[ -f "$SCHEMA" ] || {
    printf 'no schema at %s. Build it first:\n\n' "$SCHEMA" >&2
    printf '  mise run schema\n' >&2
    exit 2
}

WORKSPACE=$(awk '
    /^\[workspace\.package\]/ { in_sec = 1; next }
    /^\[/ { in_sec = 0 }
    in_sec && /^version[[:space:]]*=/ {
        sub(/^version[[:space:]]*=[[:space:]]*"/, "")
        sub(/".*$/, "")
        print; exit
    }
' "$ROOT/Cargo.toml")
[ -n "$WORKSPACE" ] || { printf 'no version under [workspace.package]\n' >&2; exit 2; }

# A tag names the version, so a prerelease zip carries the tag's own suffix.
# The part before the suffix must still be the version the binaries were built
# with, or the release ships a name its contents do not report.
if [ -n "$VERSION" ]; then
    base=${VERSION%%-*}
    if [ "$base" != "$WORKSPACE" ]; then
        printf 'the tag says %s and Cargo.toml says %s.\n' "$base" "$WORKSPACE" >&2
        printf 'Bump the workspace version, then tag what it names.\n' >&2
        exit 1
    fi
else
    VERSION=$WORKSPACE
fi

# Clear only what a previous run of this script wrote, so a mistyped output
# directory cannot take anything else with it.
mkdir -p "$OUT"
rm -rf "$OUT/.writedir"
rm -f "$OUT/lua-dcsbridge.dll" "$OUT/dcsb.exe" "$OUT/SHA256SUMS"
rm -f "$OUT"/write-directory-*.zip

# Cargo rejects a hyphen in a library target name, so the broker lands as
# lua_dcsbridge.dll and is renamed on the way out.
copy() {
    [ -f "$BUILD/$1" ] || { printf 'the build produced no %s\n' "$1" >&2; exit 1; }
    cp "$BUILD/$1" "$OUT/$2"
}
copy lua_dcsbridge.dll lua-dcsbridge.dll
copy dcsb.exe dcsb.exe

# A DLL built without the dcs-lua feature, or one whose import library resolved
# nothing, links and collects exactly like a correct one and carries no lua.dll
# in its import table. Read the table rather than trusting the build.
if LC_ALL=C grep -qa 'lua\.dll' "$OUT/lua-dcsbridge.dll"; then
    printf 'lua-dcsbridge.dll imports lua.dll\n'
else
    printf 'lua-dcsbridge.dll names no lua.dll, so it binds none of DCS Lua.\n' >&2
    printf 'Check the dcs-lua feature and vendor/lua/lua.def.\n' >&2
    exit 1
fi

# DCS loads the broker with package.loadlib and an explicit path, asking for
# luaopen_dcsbridge by name. A DLL that exports nothing stages exactly like a
# correct one, and the next thing to notice would be DCS itself, at the far end
# of an install. Read the export table here instead.
#
# llvm-readobj reads the real table, and the cross-build already installs LLVM
# for the import library. Without it, fall back to the name in the export
# directory's bytes, which is the strength of the lua.dll check above.
OPENER=luaopen_dcsbridge
READOBJ=
for candidate in llvm-readobj /usr/lib/llvm-*/bin/llvm-readobj \
    /opt/homebrew/opt/llvm/bin/llvm-readobj /usr/local/opt/llvm/bin/llvm-readobj
do
    if command -v "$candidate" >/dev/null 2>&1; then READOBJ=$candidate; break; fi
done

if [ -n "$READOBJ" ]; then
    FOUND=$("$READOBJ" --coff-exports "$OUT/lua-dcsbridge.dll")
    READ_AS="its export table"
elif LC_ALL=C grep -qa "$OPENER" "$OUT/lua-dcsbridge.dll"; then
    FOUND=$OPENER
    READ_AS="its bytes, with no llvm-readobj here to read the table"
else
    FOUND=
    READ_AS="its bytes, with no llvm-readobj here to read the table"
fi

case "$FOUND" in
    *"$OPENER"*)
        printf 'lua-dcsbridge.dll exports %s, read from %s\n' "$OPENER" "$READ_AS"
        ;;
    *)
        printf 'lua-dcsbridge.dll exports no %s, read from %s.\n' "$OPENER" "$READ_AS" >&2
        printf 'package.loadlib asks for that name, so DCS would load nothing.\n' >&2
        printf 'Check the no_mangle opener in crates/lua-module.\n' >&2
        exit 1
        ;;
esac

# SPEC 13's tree, so installing is one extraction over the write directory.
# The broker and the schema have a home there so far; the Lua files join them
# as they are built. The CLI runs outside DCS and ships as a loose asset.
ZIP="write-directory-$VERSION.zip"
STAGE=$OUT/.writedir
mkdir -p "$STAGE/Mods/services/DCSBridge/bin"
cp "$OUT/lua-dcsbridge.dll" "$STAGE/Mods/services/DCSBridge/bin/"
cp "$SCHEMA" "$STAGE/Mods/services/DCSBridge/schema.pb"
(cd "$STAGE" && zip -qr "../$ZIP" .)
rm -rf "$STAGE"

# BSD ships shasum and GNU ships sha256sum, and a stock machine has one.
if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$@"; }
elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$@"; }
else
    printf 'no sha256sum and no shasum\n' >&2
    exit 2
fi

(cd "$OUT" && sha256 lua-dcsbridge.dll dcsb.exe "$ZIP" > SHA256SUMS)

printf '\nstaged %s\n' "${OUT#"$ROOT"/}"
ls -l "$OUT"
cat "$OUT/SHA256SUMS"
