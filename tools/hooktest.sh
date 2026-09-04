#!/bin/sh
# Run each hook in .claude/hooks against a payload and check its exit code.
#
# A hook is a guard, and a guard that silently passes everything is worse
# than none. This feeds each one the payloads it exists to refuse and the
# ones it must let through, and fails on the first surprise.
#
# The hooks that read the repository run against this checkout, so the cases
# that depend on its state (a dirty tree, the current branch) are written to
# hold either way.
#
# POSIX sh only. Run it by hand, or through mise run docs.
set -u

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
HOOKS=.claude/hooks
export CLAUDE_PROJECT_DIR="$ROOT"

fail=0
n=0

# expect <code> <hook> <payload>
expect() {
    want=$1; hook=$2; payload=$3
    n=$((n + 1))
    out=$(printf '%s' "$payload" | sh "$HOOKS/$hook" 2>/dev/null)
    got=$?
    if [ "$got" -ne "$want" ]; then
        printf 'FAIL %-24s want %d got %d  %s\n' "$hook" "$want" "$got" "$payload" >&2
        fail=1
    fi
}

# expect_ask <hook> <payload>: exit 0 and a permission decision on stdout
expect_ask() {
    hook=$1; payload=$2
    n=$((n + 1))
    out=$(printf '%s' "$payload" | sh "$HOOKS/$hook" 2>/dev/null)
    got=$?
    case "$out" in
        *'"permissionDecision":"ask"'*) [ "$got" -eq 0 ] && return ;;
    esac
    printf 'FAIL %-24s want ask got %d  %s\n' "$hook" "$got" "$payload" >&2
    fail=1
}

bash_payload() {
    printf '{"tool_name":"Bash","tool_input":{"command":"%s"}}' "$1"
}
edit_payload() {
    printf '{"tool_name":"Edit","tool_input":{"file_path":"%s","old_string":"a","new_string":"b"}}' "$1"
}

for f in "$HOOKS"/*.sh; do
    sh -n "$f" || { printf 'FAIL %s does not parse\n' "$f" >&2; fail=1; }
done

# guard-spec-reads
expect 2 guard-spec-reads.sh '{"tool_name":"Read","tool_input":{"file_path":"docs/specs/bridge/spec.md"}}'
expect 2 guard-spec-reads.sh '{"tool_name":"Read","tool_input":{"file_path":"C:\\x\\docs\\specs\\bridge\\spec.md","limit":401}}'
expect 0 guard-spec-reads.sh '{"tool_name":"Read","tool_input":{"file_path":"docs/specs/bridge/spec.md","offset":10,"limit":100}}'
expect 0 guard-spec-reads.sh '{"tool_name":"Read","tool_input":{"file_path":"docs/plan/plan.md"}}'
expect 0 guard-spec-reads.sh '{"tool_name":"Edit","tool_input":{"file_path":"docs/specs/bridge/spec.md"}}'

# guard-frozen-writes
expect 2 guard-frozen-writes.sh "$(edit_payload docs/specs/bridge/spec.md)"
expect 2 guard-frozen-writes.sh "$(edit_payload "$ROOT/docs/specs/bridge/bridge-ledger.tsv")"
expect 2 guard-frozen-writes.sh "$(edit_payload .gitattributes)"
expect 2 guard-frozen-writes.sh '{"tool_name":"Write","tool_input":{"file_path":"C:\\r\\docs\\specs\\sim-driver-builtins\\spec.md","content":"x"}}'
expect 0 guard-frozen-writes.sh "$(edit_payload docs/plan/plan.md)"
expect 0 guard-frozen-writes.sh "$(edit_payload docs/decisions/0001-specifications-are-frozen.md)"
expect 0 guard-frozen-writes.sh "$(edit_payload crates/broker/src/ring.rs)"

if [ "$fail" -ne 0 ]; then
    printf '\nhook tests failed\n' >&2
    exit 1
fi
printf '%d hook cases pass\n' "$n"
