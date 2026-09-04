#!/bin/sh
# PreToolUse guard: refuse an edit to a frozen document or to .gitattributes.
#
# The specifications are frozen. They are the starting point, they are not
# maintained, and the build drifts from them by design. Where the build goes
# somewhere they did not anticipate, the record of that is a decision record,
# not an edit. docs/index.tsv says which documents are frozen, and every file
# under docs/specs/ counts, because the ledger and glossary beside a
# specification carry its hash.
#
# .gitattributes disables line-ending conversion. Without that every ledger
# stamp breaks on a Windows checkout.
#
# Exit 2 blocks the tool call and hands stderr to the model as the reason.

. "$(dirname -- "$0")/payload.sh"

path=$(parse file_path)
[ -n "$path" ] || exit 0

norm=$(normalize_path "$path")
root=$(project_root)
index="$root/docs/index.tsv"

why=""
case "$norm" in
    */docs/specs/*|docs/specs/*)
        why="a frozen specification" ;;
    */.gitattributes|.gitattributes)
        why=".gitattributes, which keeps the ledger stamps valid on Windows" ;;
esac

if [ -z "$why" ] && [ -f "$index" ]; then
    # Any path the index marks frozen, and any file in its directory.
    for p in $(awk -F '\t' 'NR > 1 && $3 == "yes" { print $2 }' "$index"); do
        d=${p%/*}
        case "$norm" in
            "$p"|*/"$p"|"$d"/*|*/"$d"/*) why="a frozen document"; break ;;
        esac
    done
fi

[ -n "$why" ] || exit 0

cat >&2 <<EOF
Blocked: a write to $norm, which is $why.

The specifications are frozen and are not brought up to date. Where the
build needs to go somewhere they did not anticipate, write a decision
record: copy docs/decisions/TEMPLATE.md and number it next.
docs/conventions/decision-records.md says how.

Leave .gitattributes alone for the same reason a ledger is not hand-edited:
its stamp is a hash of the bytes on disk.
EOF
exit 2
