#!/bin/sh
# Prove the assertion helpers fail when they should.
#
# The whole point of dist/ci-assert.sh is that the idiom it replaces could not
# fail, so helpers that cannot fail would be no better. Each one is checked in
# both directions: it passes on the true case and exits non-zero on the false
# one.
#
# Runs anywhere, needs nothing but a POSIX shell.

set -eu

HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

printf 'needle\n' > "$TMP/haystack"
: > "$TMP/exists"
printf '#!/bin/sh\n' > "$TMP/runnable"; chmod +x "$TMP/runnable"

pass=0
fails=0

# must_pass NAME ARGS... — the helper should succeed
must_pass() {
    name=$1; shift
    if sh -c ". '$HERE/ci-assert.sh'; $*" >/dev/null 2>&1; then
        pass=$((pass + 1))
    else
        printf 'FAIL: %s should have passed but did not\n' "$name" >&2
        fails=$((fails + 1))
    fi
}

# must_fail NAME ARGS... — the helper should exit non-zero
must_fail() {
    name=$1; shift
    if sh -c ". '$HERE/ci-assert.sh'; $*" >/dev/null 2>&1; then
        printf 'FAIL: %s should have failed but passed\n' "$name" >&2
        fails=$((fails + 1))
    else
        pass=$((pass + 1))
    fi
}

must_pass "assert_present on a file"        "assert_present '$TMP/exists'"
must_fail "assert_present on a missing file" "assert_present '$TMP/nope'"

must_pass "assert_executable on a +x file"  "assert_executable '$TMP/runnable'"
must_fail "assert_executable on a plain file" "assert_executable '$TMP/exists'"
must_fail "assert_executable on a missing file" "assert_executable '$TMP/nope'"

must_pass "assert_absent on a missing file" "assert_absent '$TMP/nope'"
must_fail "assert_absent on a file that exists" "assert_absent '$TMP/exists'"

must_pass "assert_contains when present"    "assert_contains needle '$TMP/haystack'"
must_fail "assert_contains when absent"     "assert_contains missing '$TMP/haystack'"

must_pass "assert_lacks when absent"        "assert_lacks missing '$TMP/haystack'"
must_fail "assert_lacks when present"       "assert_lacks needle '$TMP/haystack'"

must_pass "assert_command on sh"            "assert_command sh"
must_fail "assert_command on a missing one" "assert_command definitely-not-a-real-command-xyz"

must_pass "assert_no_command on a missing one" "assert_no_command definitely-not-a-real-command-xyz"
must_fail "assert_no_command on sh"         "assert_no_command sh"

# The specific regression: errexit must not be what carries the assertion.
# Under `set -e`, `! grep -q` does not abort, so a helper must exit itself.
if sh -c ". '$HERE/ci-assert.sh'
set -e
assert_lacks needle '$TMP/haystack'
echo reached" 2>/dev/null | grep -q reached; then
    printf 'FAIL: assert_lacks did not stop the script (the original bug)\n' >&2
    fails=$((fails + 1))
else
    pass=$((pass + 1))
fi

printf '\n%d checks passed, %d failed\n' "$pass" "$fails"
[ "$fails" -eq 0 ] || exit 1
echo "ok: the assertion helpers fail when they should"
