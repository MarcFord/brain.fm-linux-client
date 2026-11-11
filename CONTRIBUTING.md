# Contributing

## Before anything else

```sh
make check      # fmt, clippy -D warnings, full test suite
make smoke      # install and overlay checks against the real system
```

Both must pass. `make smoke` skips (exit 77) where it cannot run, so it is
safe on a headless box.

Found a security problem? Do not open an issue — see [`SECURITY.md`](SECURITY.md)
for how to report it privately. Everyone here is expected to follow the
[Code of Conduct](CODE_OF_CONDUCT.md).

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

It will. Prefer capturing a real request over reading the web app's bundle —
several reconstructions in this project's history were confidently wrong in
ways the bundle did not reveal. Put what you learn in a doc comment next to
the code that depends on it, and in a recorded fixture.

Mark anything inferred as unverified. A fixture built from an assumption
confirms the assumption rather than catching the bug.

## A note on verification

Several bugs here survived because a check could not have observed the
thing it claimed to check: a GUI "worked" because its process stayed alive,
a window looked missing because the wrong list was searched, a smoke test
passed against a binary that did not contain the change.

So: assert the actual outcome, and confirm your check fails when the thing
it tests is broken. A test never seen to fail is not evidence.

## Branches, CI and releases

Three workflows, split by how long you are willing to wait.

| Trigger | Workflow | What runs |
|---|---|---|
| any branch, any PR | `ci.yml` | fmt, clippy, tests, D-Bus contract, layering, MSRV, shell scripts |
| push to `main` or `testing` | `full.yml` | all of the above **plus** both front-ends, smoke checks, staged installs for every GUI selection, distro dependency resolution, real `.deb`/`.rpm` build and install |
| tag `v*` | `release.yml` | the gate again, then packages, checksums, and a published GitHub release |

`full.yml` *calls* `ci.yml` rather than repeating it, so the protected-branch
suite is a strict superset by construction instead of by two lists someone
has to keep in step.

`testing` exists to rehearse a release. It is treated identically to `main`
on purpose: a green `testing` has to mean the same thing as a green `main`,
or it is not a rehearsal.

Both branches are protected and require the **Full suite** checks, which is
why `full.yml` also triggers on pull requests into them: a required check
that only ran *after* a merge could never be satisfied by a PR, and nothing
would ever merge. A PR into `main` or `testing` therefore runs the fast gate
twice — once standalone, once nested inside the full suite — which is cheap
with caching and keeps the two workflows independent of each other.

Force pushes and branch deletion are blocked on both. Administrators are not
included in the restrictions, so the maintainer can still push directly; flip
`enforce_admins` if that should change.

`full.yml` also runs weekly. Distro package names drift underneath us, and
that failure is worth seeing on a Monday rather than inside someone's first
install attempt.

### The changelog

Anything a user would notice goes in `CHANGELOG.md` under `## [Unreleased]`,
in the same commit as the change. Not refactors or test-only work — things
someone running this would care about: new behaviour, changed behaviour, a
fixed bug they might have hit, a new limitation.

Note the `BrainFM1` interface version when it moves. Third-party clients
feature-detect on it, so it is as much a part of the public surface as the
release number.

It is not optional, and not on an honour system: `release.yml` reads this
file for its release notes and **refuses a tag with no section for its
version**, in the first and cheapest job. `dist/changelog-section.sh` is what
extracts a section, and `make check-scripts` keeps it honest.

### Cutting a release

```sh
# 1. move the Unreleased entries under a new `## [x.y.z] - YYYY-MM-DD` heading
#    and add the link references at the foot of the file
# 2. bump the workspace version in Cargo.toml to match
# 3. commit, merge to main
git tag v0.2.0 && git push origin v0.2.0
```

The tag is the release. `release.yml` refuses immediately if the tag and
`Cargo.toml` disagree, because packages whose filenames contradict the
release they hang off are worse than no release. A tag containing a hyphen
(`v0.2.0-rc1`) publishes as a prerelease. Re-running the workflow by hand
for an existing tag replaces its assets rather than failing.

### Writing assertions in CI

Use the helpers in `dist/ci-assert.sh`, not bare shell:

```sh
. dist/ci-assert.sh
assert_lacks 'bfm-gtk' /tmp/contents.txt "the daemon package must stay desktop-free"
```

They exist because of a trap that had silently disabled several of this
project's packaging checks. POSIX exempts a pipeline preceded by `!` from
errexit, so this:

```sh
set -e; ! grep -q 'bfm-gtk' contents.txt
```

does **nothing** when grep finds the thing — the negation returns 1, the
shell does not abort, and the step passes. It looks exactly like a working
check. `dist/ci-assert-test.sh` proves each helper fails when it should, and
`make check-scripts` runs it.

## Commits

Explain *why*, and record what was ruled out — the history here is meant to
be readable by someone debugging the same thing later.

Do not add tool or AI attribution trailers.
