#!/usr/bin/env bash
# Prints Markdown release notes for <tag>: its commits since the previous published release, grouped by type.
# Tags whose release never got published (a failed run) are skipped, so their commits still show up.
# Usage: scripts/release-notes.sh v0.2.0
set -euo pipefail

TAG="${1:?usage: scripts/release-notes.sh <tag>}"
PUBLISHED="$(gh release list --exclude-drafts --limit 100 --json tagName --jq '.[].tagName' 2>/dev/null || true)"
PREVIOUS=""
for candidate in $(git tag --merged "$TAG^" --sort=-creatordate 2>/dev/null); do
    if grep -qxF "$candidate" <<<"$PUBLISHED"; then
        PREVIOUS="$candidate"
        break
    fi
done
RANGE="${PREVIOUS:+$PREVIOUS..}$TAG"
SUBJECTS="$(git log --no-merges --format=%s "$RANGE")"

# "feat(app): add X" -> "- **app:** add X"; a "!" before the colon still matches the type.
format() {
    sed -E -e 's/^[a-z]+\(([^)]*)\)!?: /- **\1:** /' -e 's/^[a-z]+!?: /- /' -e '/^- /!s/^/- /'
}

section() {
    local title="$1" pattern="$2" lines
    lines="$(grep -E "^($pattern)(\([^)]*\))?!?: " <<<"$SUBJECTS" | format || true)"
    if [[ -n "$lines" ]]; then
        printf '## %s\n\n%s\n\n' "$title" "$lines"
    fi
}

section "Features" "feat"
section "Fixes" "fix"
section "Performance" "perf"
section "Refactoring" "refactor"
# Everything else (ci, test, build, chore, docs and non-conventional subjects), except the ODD logbook.
OTHER="$(grep -vE "^(feat|fix|perf|refactor)(\([^)]*\))?!?: |^docs\(odd\)" <<<"$SUBJECTS" | grep -v '^$' | format || true)"
if [[ -n "$OTHER" ]]; then
    printf '## Other changes\n\n%s\n' "$OTHER"
fi
