#!/bin/sh
# Miri over the ring's and the fan-out's tests, on the nightly it installs.
#
# Miri reads the unsafe blocks the way Loom reads the atomics: it runs the
# ring and the writer under an interpreter that checks every access against
# the ownership argument written beside it. It needs a nightly toolchain with
# the miri component, which is installed here if it is missing and left in
# place; the pinned toolchain in rust-toolchain.toml is untouched.
#
# Two test filters rather than the whole library, because a state test reads
# the schema off disk, which Miri's isolation refuses. A test that runs
# threads against the clock opts out with cfg_attr(miri, ignore), since the
# interpreter turns a paced thread into minutes per step.
#
# CI's Linux job runs this script, and so does `mise run miri`, so the two
# cannot drift. POSIX sh only.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

if ! rustup run nightly cargo miri --version >/dev/null 2>&1; then
    rustup toolchain install nightly --component miri --profile minimal
fi
rustup run nightly cargo miri setup

exec rustup run nightly cargo miri test -p dcsbridge-broker --no-default-features --lib -- ring:: fanout::
