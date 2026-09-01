#!/bin/sh
# SPEC 8.4's compatibility check: compare the .proto tree against the previous
# release and refuse a change that breaks a consumer already deployed.
#
# Field numbers are permanent and a removed one is reserved rather than reused.
# The schema hash a consumer compares across the handshake reports a mismatch
# only as a warning (SPEC 5.2), so nothing at run time refuses an incompatible
# build. This is where that is caught.
#
# The comparison is against the newest release tag rather than against main.
# A release is what a consumer has, and two changes that are each compatible
# with the commit before them can still be incompatible with the last release
# between them.
#
# A tag carrying a hyphenated suffix is the prerelease channel (release.yml),
# so it is not a release and this skips it.
#
# The baseline is the tag's own .proto tree, read straight out of git. That is
# the same source the release compiled its schema.pb from, and it needs no
# published asset and no network.
#
# buf.yaml sets no breaking rules, so this runs buf's default FILE category.
# It is the strictest, and it catches the move that matters most here: a topic
# id is <package>.<Message> (SPEC 5.2), so renaming a message or moving it to
# another package breaks every consumer subscribed to it while leaving the wire
# bytes compatible.
#
# Usage: sh tools/schema-breaking.sh [tag]
#
# POSIX sh only. Needs buf and git; mise.toml pins the buf version.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

command -v buf >/dev/null 2>&1 || {
    printf 'no buf on PATH. mise.toml pins the version:\n\n' >&2
    printf '  mise install\n  mise exec -- sh tools/schema-breaking.sh\n' >&2
    exit 2
}

TAG=${1:-}
if [ -z "$TAG" ]; then
    TAG=$(git tag --list 'v*' --sort=-v:refname | awk '!/-/ { print; exit }')
fi

if [ -z "$TAG" ]; then
    if [ "$(git rev-parse --is-shallow-repository)" = true ]; then
        printf 'no release tag, and this clone is shallow.\n' >&2
        printf 'Fetch the tags before trusting that: actions/checkout wants\n' >&2
        printf 'fetch-depth: 0.\n' >&2
        exit 1
    fi
    printf 'no release tag yet, so there is nothing to compare against.\n'
    printf 'The first v* tag becomes the baseline for every change after it.\n'
    exit 0
fi

git rev-parse --verify --quiet "refs/tags/$TAG" >/dev/null || {
    printf 'no tag %s in this checkout.\n' "$TAG" >&2
    printf 'A shallow clone carries no tags: actions/checkout wants\n' >&2
    printf 'fetch-depth: 0.\n' >&2
    exit 1
}

# A release from before the schema existed has nothing to compare. buf reports
# that as "had no .proto files", which reads like a broken configuration rather
# than the expected state, so say what it is.
if ! git ls-tree -r --name-only "$TAG" -- proto | grep -q '\.proto$'; then
    printf '%s carries no .proto tree, so there is nothing to compare\n' "$TAG"
    printf 'against. The first release that ships a schema becomes the\n'
    printf 'baseline for every change after it.\n'
    exit 0
fi

git ls-tree -r --name-only "$TAG" -- buf.yaml | grep -q . || {
    printf '%s carries a .proto tree and no buf.yaml.\n' "$TAG" >&2
    exit 1
}

printf 'comparing against %s\n' "$TAG"
buf breaking --against ".git#tag=$TAG"
printf 'no change breaks a consumer built against %s\n' "$TAG"
