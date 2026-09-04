# Sourced by every hook: reads the JSON payload from stdin into $payload and
# defines parse, which returns one top-level or nested field by key.
#
# No jq. It is not installed by default on macOS or Windows, so the payload is
# scanned with awk. The first occurrence of the key wins, which is the one in
# tool_input: the harness writes that before tool_response, and a key echoed
# inside a command's output arrives escaped and later.
#
# A string value is decoded: \n, \t, \r, \" and \\ become the characters they
# stand for, so a multi-line Bash command reads as the lines it was. A number
# or a boolean is returned as written.
#
# Usage, from a hook in this directory:
#
#     . "$(dirname -- "$0")/payload.sh"
#     cmd=$(parse command)
#     active=$(parse stop_hook_active)
#
# A hook that needs the repository runs from it: the harness starts a hook in
# the session's working directory, which is not always the checkout.

payload=$(cat)

parse() {
    printf '%s' "$payload" | awk -v key="$1" '
        # One record for the whole input. RS is a single character in every
        # POSIX awk, and \001 does not occur in JSON.
        BEGIN { RS = "\001" }
        {
            pat = "\"" key "\"[ \t\r\n]*:[ \t\r\n]*"
            if (!match($0, pat)) exit
            rest = substr($0, RSTART + RLENGTH)
            if (substr(rest, 1, 1) != "\"") {
                # Number, true, false or null.
                match(rest, /^[A-Za-z0-9.+-]+/)
                printf "%s", substr(rest, 1, RLENGTH)
                exit
            }
            out = ""
            n = length(rest)
            for (i = 2; i <= n; i++) {
                ch = substr(rest, i, 1)
                if (ch == "\\") {
                    i++
                    e = substr(rest, i, 1)
                    if (e == "n") out = out "\n"
                    else if (e == "t") out = out "\t"
                    else if (e == "r") out = out "\r"
                    else if (e == "u") { out = out "?"; i += 4 }
                    else out = out e
                    continue
                }
                if (ch == "\"") break
                out = out ch
            }
            printf "%s", out
        }
    '
}

# The checkout. CLAUDE_PROJECT_DIR is set by the harness; the fallback walks
# up from this file for a hook run by hand.
project_root() {
    if [ -n "${CLAUDE_PROJECT_DIR:-}" ]; then
        printf '%s' "$CLAUDE_PROJECT_DIR"
    else
        CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd
    fi
}

# Forward slashes, squeezed, so one pattern covers both platforms and a
# doubled separator cannot slip a path past it.
normalize_path() {
    printf '%s' "$1" | tr '\\' '/' | tr -s '/'
}
