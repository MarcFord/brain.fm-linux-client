# Contributing

## Before anything else

```sh
make check      # fmt, clippy -D warnings, full test suite
make smoke      # install and overlay checks against the real system
```

Both must pass. `make smoke` skips (exit 77) where it cannot run, so it is
safe on a headless box.

## Three rules that are not negotiable

**No test may touch the live API.** Everything runs against recorded
fixtures in `crates/bfm-api/tests/fixtures/`. CI fails the build if a test
mentions `api.brain.fm`. The API is undocumented and can change without
warning; tests that depend on it are flaky by construction and mutate a real
account.

**No front-end may depend on `bfm-api` or `bfm-player`.** The CLI and both
GUIs talk to the daemon over D-Bus and nothing else. The moment one can
reach Brain.fm directly, the daemon stops being the single source of truth
and every third-party client becomes second-class. Enforced by
`crates/bfm-ui-core/tests/frontends.rs`.

**`dev.marcford.BrainFM1` is additive-only.** New methods, properties and
signals may appear; existing signatures never change meaning. Bump
`INTERFACE_VERSION` for each addition so clients can feature-detect. A
breaking change means `BrainFM2`, served alongside the old one.

## When the API surprises you

It will. Record what you actually observed in `docs/api.md`, and prefer
capturing a real request over reading the web app's bundle — several
reconstructions in this project's history were confidently wrong in ways
the bundle did not reveal.

Mark anything inferred as unverified. A fixture built from an assumption
confirms the assumption rather than catching the bug.

## A note on verification

Several bugs here survived because a check could not have observed the
thing it claimed to check: a GUI "worked" because its process stayed alive,
a window looked missing because the wrong list was searched, a smoke test
passed against a binary that did not contain the change.

So: assert the actual outcome, and confirm your check fails when the thing
it tests is broken. A test never seen to fail is not evidence.

## Commits

Explain *why*, and record what was ruled out — the history here is meant to
be readable by someone debugging the same thing later.
