#!/bin/sh
# Check that every Lua entry that raises owns nothing, in the product build.
#
# A raise is a longjmp out of lua.dll, and on x64 Windows a longjmp is an
# unwind through every frame it crosses. The one shape known to survive it
# is an entry frame with nothing to destroy: the work happens in a callee
# that returns, and the entry calls lua_error holding nothing. The source
# says so and the optimizer is free to undo it by inlining the callee, which
# it did once, and DCS exited without a report at the first refusal. The
# doc on push_error in crates/lua-module/src/lib.rs holds the argument.
#
# The check reads the assembly of the product target: for each raising
# entry, MSVC-style exception handling names every destructor funclet after
# the function it belongs to, so an entry that owns nothing has none. The
# stack allocation is printed beside it as the other symptom, not enforced.
#
# With no argument, builds the module for x86_64-pc-windows-msvc in release
# with its default features and as the cdylib that ships, since the crate
# type decides what the optimizer may internalize and inline, and keeps
# the assembly beside the build. That is the product build, so it needs
# what the product build needs: the import-library tool and, off a Windows
# host, cargo-xwin. With one argument, reads that assembly file instead.
#
# POSIX sh and awk only. Exits 1 on a finding, 2 when nothing could be read.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TARGET=x86_64-pc-windows-msvc

# The v0-mangled tail of each entry: the module and the function, each
# with its length in front. begin and begin_to raise with luaL_error and
# are held to the same shape.
ENTRIES='configure9configure schema6schema register7classes register6routes register4caps register7replies put5begin put8begin_to'

if [ $# -ge 1 ]; then
    ASM=$1
else
    # A Windows host links with its own toolchain; any other host links the
    # way the release does, through cargo-xwin. The assembly a previous run
    # left is removed first, so a build that emits nothing is not read as
    # the old one.
    case "$(uname -s)" in
        MINGW* | MSYS* | CYGWIN* | Windows_NT) CARGO='cargo rustc' ;;
        *) CARGO='cargo xwin rustc' ;;
    esac
    # A cdylib's assembly carries no hash in its name; an rlib's would.
    ASM=$ROOT/target/$TARGET/release/deps/lua_dcsbridge.s
    rm -f "$ASM"
    (cd "$ROOT" && $CARGO -p lua-dcsbridge --release --target "$TARGET" -- --emit=asm)
fi

[ -n "$ASM" ] && [ -f "$ASM" ] || {
    printf 'no assembly to read: expected target/%s/release/deps/lua_dcsbridge.s\n' "$TARGET" >&2
    exit 2
}

status=0
for entry in $ENTRIES; do
    # The function's own label, and the first stack allocation after it.
    alloc=$(awk -v s="$entry" '
        $0 ~ "^_RNv[A-Za-z0-9_]*" s ":$" { found = 1 }
        found && /\.seh_stackalloc/ { print $2; exit }
        found && /\.seh_endproc/ { print "0"; exit }
    ' "$ASM")
    if [ -z "$alloc" ]; then
        printf 'MISSING  %-22s not in %s\n' "$entry" "$ASM"
        status=1
        continue
    fi
    # Every destructor funclet the entry owns: "?dtor$N@?0?<entry>@4HA":
    dtors=$(awk -v s="$entry" '
        $0 ~ "^\"\\?dtor\\$[0-9]+@\\?0\\?_RNv[A-Za-z0-9_]*" s "@4HA\":$" { n++ }
        END { print n + 0 }
    ' "$ASM")
    if [ "$dtors" -eq 0 ]; then
        printf 'ok       %-22s stack %-4s destructors 0\n' "$entry" "$alloc"
    else
        printf 'OWNS     %-22s stack %-4s destructors %s\n' "$entry" "$alloc" "$dtors"
        status=1
    fi
done

if [ "$status" -ne 0 ]; then
    printf '\nAn entry that raises owns a value with a destructor. Keep the work in a\n'
    printf 'callee marked #[inline(never)]; see push_error in crates/lua-module/src/lib.rs.\n'
    exit 1
fi
printf 'every raising entry owns nothing\n'
