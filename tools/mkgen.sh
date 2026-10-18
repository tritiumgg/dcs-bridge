#!/bin/sh
# Generate the bridge's Lua files from the .proto tree, through buf.
#
# buf.gen.yaml runs protoc-gen-dcsbridge-lua over every file under proto/
# and writes SimDriver.gen.lua and HookDriver.gen.lua under target/gen. Each
# carries the SHA-256 of target/schema.pb, which the broker hashes from the
# bytes the hook driver hands it, so this checks the two agree and fails
# when they do not. ADR 0039.
#
# Needs target/schema.pb, so tools/mkschema.sh runs first, and the plugin
# built by cargo. mise run generate does both.
#
# POSIX sh only. Needs buf; mise.toml pins the version.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SCHEMA=$ROOT/target/schema.pb
OUT=$ROOT/target/gen

die() {
    printf '%s\n' "$*" >&2
    exit 1
}

command -v buf >/dev/null 2>&1 || {
    printf 'no buf on PATH. mise.toml pins the version:\n\n' >&2
    printf '  mise install\n  mise run generate\n' >&2
    exit 2
}

[ -f "$SCHEMA" ] || die "no ${SCHEMA#"$ROOT"/}. Run sh tools/mkschema.sh first."

# sha256sum on Linux and Git Bash, shasum on macOS. Both are stock.
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        die "no sha256sum and no shasum on PATH"
    fi
}

rm -rf "$OUT"
(cd "$ROOT" && buf generate)

WANT=$(sha256_of "$SCHEMA")
for name in SimDriver.gen.lua HookDriver.gen.lua; do
    file=$OUT/$name
    [ -f "$file" ] || die "buf generate wrote no ${file#"$ROOT"/}"
    grep -q "^[a-z]*\.schema_sha256 = '$WANT'\$" "$file" ||
        die "${file#"$ROOT"/} does not carry the hash of ${SCHEMA#"$ROOT"/}, $WANT"
    printf 'wrote %s\n' "${file#"$ROOT"/}"
done
