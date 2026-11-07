# shellcheck shell=sh
# Assertion helpers for CI steps. Sourced, so it has no shebang:
#   . dist/ci-assert.sh
#
# This file exists because of a trap that had silently disabled several of
# this project's packaging checks. The idiom was, on one line:
#
#   set -e; ! grep -q 'bfm-gtk' contents.txt
#
# meaning "assert the GUI binary is not in this package".
#
# POSIX exempts a pipeline preceded by `!` from errexit, so when grep *does*
# find the thing, the negation returns 1, the shell does not abort, and the
# step passes. Every assertion written that way is decorative. Worse, it looks
# exactly like a working check.
#
# These helpers never rely on errexit: each one exits explicitly, and emits a
# GitHub Actions ::error:: annotation so a failure says what it was.
#
# Covered by dist/ci-assert-test.sh, which proves each one both passes when it
# should and fails when it should.

fail() { printf '::error::%s\n' "$*" >&2; exit 1; }

# assert_present PATH [NOTE]
assert_present() {
    test -e "$1" || fail "missing: $1${2:+ ($2)}"
    printf '  ok: present  %s\n' "$1"
}

# assert_executable PATH [NOTE]
assert_executable() {
    test -x "$1" || fail "not executable (or missing): $1${2:+ ($2)}"
    printf '  ok: exec     %s\n' "$1"
}

# assert_absent PATH [NOTE] — the case the `!` idiom got wrong
assert_absent() {
    if test -e "$1"; then
        fail "should not exist: $1${2:+ ($2)}"
    fi
    printf '  ok: absent   %s\n' "$1"
}

# assert_contains PATTERN FILE [NOTE]
assert_contains() {
    grep -q -- "$1" "$2" || fail "'$1' not found in $2${3:+ ($3)}"
    printf '  ok: contains %s\n' "$1"
}

# assert_lacks PATTERN FILE [NOTE]
assert_lacks() {
    if grep -q -- "$1" "$2"; then
        fail "'$1' must not appear in $2${3:+ ($3)}"
    fi
    printf '  ok: lacks    %s\n' "$1"
}

# assert_no_command NAME [NOTE]
assert_no_command() {
    if command -v "$1" >/dev/null 2>&1; then
        fail "'$1' is still on PATH${2:+ ($2)}"
    fi
    printf '  ok: gone     %s\n' "$1"
}

# assert_command NAME [NOTE]
assert_command() {
    command -v "$1" >/dev/null 2>&1 || fail "'$1' is not on PATH${2:+ ($2)}"
    printf '  ok: on PATH  %s\n' "$1"
}
