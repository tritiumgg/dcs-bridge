#!/bin/sh
# Locate and retrieve specification text through the ledger.
# POSIX sh and awk only. No jq, no python, no GNU-only flags.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
INDEX="$ROOT/docs/index.tsv"

usage() {
    cat <<'EOF'
usage: tools/ledger.sh <command> [arguments]

  codes                        list document codes and paths
  sections CODE                heading tree with line ranges and sizes
  read CODE SECTION            print one section, by number or exact heading
  find CODE PATTERN            ledger rows whose subject, claim or section matches
  subjects CODE                every distinct subject, with row counts
  show CODE ANCHOR [B] [A]     print B lines before and A lines after an anchor
  stamp [CODE]                 compare each ledger stamp against the living file
  lint [CODE]                  full check: stamp, anchors, sections, glossary join

A frozen document must not change, so a stamp mismatch there fails. The plan is
not frozen, so a mismatch reports STALE and passes.

CODE is a column-one value from docs/index.tsv. Omit it where optional to
run over every document.

PATTERN and SECTION are plain substrings, not regular expressions.
EOF
}

die() { printf '%s\n' "$*" >&2; exit 2; }

# Resolve a document code to an absolute spec path.
doc_path() {
    p=$(awk -F'\t' -v c="$1" 'NR>1 { sub(/\r$/, ""); if ($1 == c) { print $2; exit } }' "$INDEX")
    [ -n "$p" ] || die "unknown document code: $1 (try: tools/ledger.sh codes)"
    printf '%s/%s' "$ROOT" "$p"
}

# Ledger and glossary live beside the spec, sharing its stem.
led_path() { printf '%s' "$(doc_path "$1" | sed 's/\.md$/-ledger.tsv/')"; }
glo_path() { printf '%s' "$(doc_path "$1" | sed 's/\.md$/-glossary.tsv/')"; }

all_codes() { awk -F'\t' 'NR>1 { sub(/\r$/, ""); if ($1 != "") print $1 }' "$INDEX"; }

# A frozen document must never change, so a stamp mismatch is tampering. The
# plan changes weekly and says so in its own first paragraph, so a mismatch
# there is expected and does not fail anything.
is_frozen() {
    awk -F'\t' -v c="$1" 'NR>1 { sub(/\r$/, ""); if ($1 == c) { print ($3 == "yes" ? "yes" : "no"); exit } }' "$INDEX"
}

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

cmd_codes() {
    printf '%-6s %s\n' CODE PATH
    awk -F'\t' 'NR>1 { sub(/\r$/, ""); if ($1 != "") printf "%-6s %s\n", $1, $2 }' "$INDEX"
}

cmd_sections() {
    [ $# -ge 1 ] || die "sections needs a document code"
    awk '
        { sub(/\r$/, "") }
        /^```/ { fence = !fence; next }
        !fence && /^#+ / {
            if (n > 0) size[n] = NR - start[n] - 1
            n++
            depth[n] = index($0, " ") - 1
            head[n] = substr($0, depth[n] + 2)
            start[n] = NR
        }
        END {
            size[n] = NR - start[n]
            for (i = 1; i <= n; i++) {
                pad = ""
                for (j = 2; j < depth[i]; j++) pad = pad "  "
                printf "%6d  %5d lines  %s%s\n", start[i], size[i], pad, head[i]
            }
        }
    ' "$(doc_path "$1")"
}

cmd_read() {
    [ $# -ge 2 ] || die "read needs a document code and a section"
    awk -v want="$2" '
        { sub(/\r$/, "") }
        /^```/ { fence = !fence }
        !fence && /^#+ / {
            h = $0; sub(/^#+ +/, "", h)
            d = index($0, " ") - 1
            if (printing && d <= mydepth) { printing = 0 }
            if (!printing && (h == want || index(h, want) == 1)) {
                printing = 1; mydepth = d; found = 1
            }
        }
        printing { print }
        END { if (!found) { print "no section matching: " want > "/dev/stderr"; exit 2 } }
    ' "$(doc_path "$1")"
}

cmd_find() {
    [ $# -ge 2 ] || die "find needs a document code and a pattern"
    awk -F'\t' -v pat="$2" '
        { sub(/\r$/, "") }
        FNR > 4 && NF >= 6 {
            if (index($1, pat) || index($4, pat) || index($5, pat)) {
                printf "subject : %s\n", $1
                printf "kind    : %s%s\n", $2, ($3 == "" ? "" : "   status: " $3)
                printf "claim   : %s\n", $4
                printf "section : %s\n", $5
                printf "anchor  : %s\n\n", $6
                hits++
            }
        }
        END { if (!hits) print "no ledger row matches: " pat > "/dev/stderr" }
    ' "$(led_path "$1")"
}

cmd_subjects() {
    [ $# -ge 1 ] || die "subjects needs a document code"
    awk -F'\t' '
        { sub(/\r$/, "") }
        FNR > 4 && NF >= 6 { c[$1]++ }
        END { for (s in c) printf "%4d  %s\n", c[s], s }
    ' "$(led_path "$1")" | sort -k2
}

cmd_show() {
    [ $# -ge 2 ] || die "show needs a document code and an anchor"
    before=${3:-4}
    after=${4:-12}
    awk -v a="$2" -v b="$before" -v f="$after" '
        { sub(/\r$/, ""); line[NR] = $0; if (index($0, a)) { hit = NR; n++ } }
        END {
            if (n == 0) { print "anchor not found" > "/dev/stderr"; exit 2 }
            if (n > 1)  { print "anchor is not unique: " n " occurrences" > "/dev/stderr"; exit 2 }
            s = hit - b; if (s < 1) s = 1
            e = hit + f; if (e > NR) e = NR
            for (i = s; i <= e; i++) printf "%5d%s %s\n", i, (i == hit ? " >" : "  "), line[i]
        }
    ' "$(doc_path "$1")"
}

stamp_one() {
    spec=$(doc_path "$1")
    led=$(led_path "$1")
    glo=$(glo_path "$1")
    actual=$(sha256_of "$spec")
    rc=0
    for f in "$led" "$glo"; do
        recorded=$(awk 'NR<=3 { sub(/\r$/, ""); if (sub(/^# sha256: /, "")) { print; exit } }' "$f")
        named=$(awk 'NR<=3 { sub(/\r$/, ""); if (sub(/^# spec: /, "")) { print; exit } }' "$f")
        base=$(basename "$spec")
        rel=${f#"$ROOT"/}
        if [ -z "$recorded" ]; then
            printf 'MISSING   %s  (no sha256 stamp line)\n' "$rel"; rc=1
        elif [ "$recorded" != "$actual" ]; then
            if [ "$(is_frozen "$1")" = yes ]; then
                printf 'MISMATCH  %s  (a frozen document changed)\n' "$rel"; rc=1
            else
                printf 'STALE     %s  (%s changes; regenerate its rows when convenient)\n' "$rel" "$1"
            fi
        elif [ "$named" != "$base" ]; then
            printf 'RENAMED   %s  (stamp names %s, file is %s)\n' "$rel" "$named" "$base"; rc=1
        else
            printf 'MATCH     %s\n' "$rel"
        fi
    done
    return $rc
}

cmd_stamp() {
    rc=0
    if [ $# -ge 1 ]; then
        stamp_one "$1" || rc=1
    else
        for c in $(all_codes); do stamp_one "$c" || rc=1; done
    fi
    return $rc
}

lint_one() {
    code=$1
    spec=$(doc_path "$code")
    led=$(led_path "$code")
    glo=$(glo_path "$code")
    printf '== %s (%s)\n' "$code" "${spec#"$ROOT"/}"
    rc=0
    stamp_one "$code" | sed 's/^/   /' || rc=1

    # Anchors: present, unique, and inside the section the row names.
    out=$(awk -F'\t' -v spec="$spec" '
        function load(  l) {
            while ((getline l < spec) > 0) {
                sub(/\r$/, "", l)
                nl++; text[nl] = l
                if (l ~ /^```/) fence = !fence
                if (!fence && l ~ /^#+ /) {
                    h = l; sub(/^#+ +/, "", h)
                    nh++; hline[nh] = nl; hname[nh] = h
                }
            }
            close(spec)
        }
        BEGIN { load() }
        { sub(/\r$/, "") }
        FNR <= 4 { next }
        NF < 6 { printf "SHORT-ROW line %d: %d fields, want 6\n", FNR, NF; next }
        {
            subject = $1; kind = $2; status = $3; section = $5; anchor = $6
            if (kind !~ /^(definition|interface|behavior|constraint|dependency|goal|nongoal)$/)
                printf "BAD-KIND     %s: %s\n", subject, kind
            if (status != "" && status !~ /^(UNVERIFIED|PROPOSED)$/)
                printf "BAD-STATUS   %s: %s\n", subject, status
            hits = 0; where = 0
            for (i = 1; i <= nl; i++) if (index(text[i], anchor)) { hits++; where = i }
            if (hits == 0)      { printf "MISSING-ANCHOR  %s: %s\n", subject, substr(anchor, 1, 60); next }
            if (hits > 1)       { printf "DUPLICATE-ANCHOR (%d) %s: %s\n", hits, subject, substr(anchor, 1, 60); next }
            ok = 0
            for (i = nh; i >= 1; i--) if (hline[i] <= where && hname[i] == section) { ok = 1; break }
            if (!ok) {
                owner = "(none)"
                for (i = 1; i <= nh; i++) if (hline[i] <= where) owner = hname[i]
                printf "SECTION-MISMATCH %s: row says [%s], anchor sits in [%s]\n", subject, section, owner
            }
            rows++
        }
        END { printf "checked %d ledger rows\n", rows }
    ' "$led")
    printf '%s\n' "$out" | sed 's/^/   /'
    printf '%s\n' "$out" | grep -q -E '^(MISSING-ANCHOR|DUPLICATE-ANCHOR|SECTION-MISMATCH|SHORT-ROW|BAD-KIND|BAD-STATUS)' && rc=1

    # Glossary: one row per term, every subject joins a ledger subject.
    out=$(awk -F'\t' '
        { sub(/\r$/, "") }
        NR == FNR { if (FNR > 4 && NF >= 6) lsub[$1] = 1; next }
        FNR <= 4 { next }
        NF < 4 { printf "SHORT-GLOSSARY-ROW line %d: %d fields, want 4\n", FNR, NF; next }
        {
            if (seen[$1]++) printf "DUPLICATE-TERM  %s\n", $1
            if (!($2 in lsub)) printf "ORPHAN-SUBJECT  term [%s] names subject [%s], which has no ledger row\n", $1, $2
            terms++
        }
        END { printf "checked %d glossary rows\n", terms }
    ' "$led" "$glo")
    printf '%s\n' "$out" | sed 's/^/   /'
    printf '%s\n' "$out" | grep -q -E '^(DUPLICATE-TERM|ORPHAN-SUBJECT|SHORT-GLOSSARY-ROW)' && rc=1

    return $rc
}

cmd_lint() {
    rc=0
    if [ $# -ge 1 ]; then
        lint_one "$1" || rc=1
    else
        for c in $(all_codes); do lint_one "$c" || rc=1; done
    fi
    if [ "$rc" -eq 0 ]; then printf '\nlint clean\n'; else printf '\nlint FAILED\n'; fi
    return $rc
}

[ $# -ge 1 ] || { usage; exit 2; }
sub=$1
shift
case "$sub" in
    codes)    cmd_codes ;;
    sections) cmd_sections "$@" ;;
    read)     cmd_read "$@" ;;
    find)     cmd_find "$@" ;;
    subjects) cmd_subjects "$@" ;;
    show)     cmd_show "$@" ;;
    stamp)    cmd_stamp "$@" ;;
    lint)     cmd_lint "$@" ;;
    -h|--help|help) usage ;;
    *)        die "unknown command: $sub" ;;
esac
