#!/bin/sh
# Compile the .proto tree into the FileDescriptorSet the bridge deploys.
#
# The hook driver reads Mods\services\DCSBridge\schema.pb at DCS start and
# hands the bytes to the broker once (SPEC 5.1). The broker hashes them and
# serves them back through GetSchema, and a consumer compares that hash against
# the one the handshake carries. The output is a wire artifact: two runs over
# the same tree must produce the same bytes, or the comparison reports a
# mismatch that nothing caused.
#
# Two things follow from that.
#
# The buf version is pinned in mise.toml, and moving it can move these bytes.
# buf vendors its own google/protobuf/descriptor.proto and that file is in the
# set, so a buf release that touches it changes the hash without a line of this
# project changing. Expect a hash move on a buf bump and nowhere else.
#
# Source info stays in. It is the comments, and SPEC 8.2 puts a contract in
# them: every position field states +x north, +z east, +y up, and every
# heading field states grid, true or magnetic. A consumer reads those to
# interpret the numbers, so --exclude-source-info would strip the artifact of
# what makes it readable, to save about 70KB on a file fetched once.
#
# --as-file-descriptor-set drops buf's own image metadata, leaving the plain
# descriptor set every protobuf runtime reads. Imports stay, so a consumer
# resolves the Any payload from this file alone.
#
# POSIX sh only. Needs buf; mise.toml pins the version.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
OUT=${1:-$ROOT/target/schema.pb}

command -v buf >/dev/null 2>&1 || {
    printf 'no buf on PATH. mise.toml pins the version:\n\n' >&2
    printf '  mise install\n  mise exec -- sh tools/mkschema.sh\n' >&2
    exit 2
}

mkdir -p "$(dirname "$OUT")"
(cd "$ROOT" && buf build --as-file-descriptor-set -o "$OUT")

printf 'wrote %s (%s bytes)\n' "${OUT#"$ROOT"/}" "$(wc -c < "$OUT" | tr -d ' ')"
