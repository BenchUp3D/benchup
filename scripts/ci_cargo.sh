#!/usr/bin/env bash
# Keep the full Cargo log and expose actionable failures as job annotations.
set -euo pipefail

log=$(mktemp)
trap 'rm -f "$log"' EXIT

if CARGO_TERM_COLOR=never cargo "$@" 2>&1 | tee "$log"; then
    exit 0
fi

awk '
    function escape(line) {
        gsub(/%/, "%25", line)
        sub(/\r$/, "", line)
        return line
    }
    function annotate() {
        if (message != "") print "::error::" message
        message = ""
    }
    /^error(\[[[:alnum:]]+\])?:/ || /^test .* \.\.\. FAILED$/ {
        annotate()
        if (++count > 12) exit
        message = escape($0)
        context = 10
        next
    }
    message != "" && context-- > 0 && NF {
        message = message "%0A" escape($0)
        next
    }
    { annotate() }
    END { annotate() }
' "$log"
exit 1
