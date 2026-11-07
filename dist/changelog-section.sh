#!/bin/sh
# Print one version's section from CHANGELOG.md.
#
#   dist/changelog-section.sh 0.1.0 [CHANGELOG.md]
#
# Exits 1 if there is no section for that version, which is what makes the
# changelog impossible to forget: the release workflow calls this, so a tag
# with no entry fails before anything is published.
#
# Matches a heading of the form:
#   ## [0.1.0] - 2026-11-07
#   ## [0.1.0]
#
# and ends at whichever comes first:
#   * the next `## ` heading, or
#   * the link-reference block at the foot of the file (`[0.1.0]: https://...`),
#     which is part of the document's plumbing and not of any section.
#
# Interior blank lines are preserved -- collapsing them would run paragraphs
# into the `###` headings that follow and break the rendered markdown. Only
# leading and trailing blank lines are trimmed.

set -eu

if [ $# -lt 1 ]; then
    echo "usage: $0 <version> [changelog]" >&2
    exit 2
fi

version=$1
file=${2:-CHANGELOG.md}

[ -r "$file" ] || { echo "error: cannot read $file" >&2; exit 1; }

section=$(awk -v want="$version" '
    # Collect the wanted section, then print it trimmed of blank edges.
    function emit() {
        if (n == 0) return
        first = 1; last = n
        while (first <= n && buf[first] ~ /^[[:space:]]*$/) first++
        while (last >= 1 && buf[last]  ~ /^[[:space:]]*$/) last--
        for (i = first; i <= last; i++) print buf[i]
    }

    # A version heading: ## [1.2.3] anything
    /^## \[/ {
        v = $0
        sub(/^## \[/, "", v)
        sub(/\].*$/, "", v)
        if (inside) { emit(); found = 1; exit }
        if (v == want) { inside = 1 }
        next
    }

    # The link-reference block ends the last section.
    inside && /^\[[^]]+\]:[[:space:]]/ { emit(); found = 1; exit }

    inside { buf[++n] = $0 }

    END { if (inside && !found) emit() }
' "$file")

if [ -z "$section" ]; then
    echo "error: CHANGELOG.md has no entry for version $version." >&2
    echo "Add a '## [$version] - <date>' section describing the release." >&2
    exit 1
fi

printf '%s\n' "$section"
