#!/usr/bin/env bash
# Keep the full Cargo log and expose actionable failures as job annotations.
set -euo pipefail

log=$(mktemp)
trap 'rm -f "$log"' EXIT

if CARGO_TERM_COLOR=never cargo "$@" 2>&1 | tee "$log"; then
    exit 0
fi

awk '
    /^error(\[[[:alnum:]]+\])?:/ || /^test .* \.\.\. FAILED$/ {
        gsub(/%/, "%25")
        sub(/\r$/, "")
        print "::error::" $0
        if (++count == 12) exit
    }
' "$log"
exit 1
