<!--
Thanks for the contribution. The headings below mirror what
CONTRIBUTING.md asks for; a short answer under each is enough. Delete any
that do not apply.
-->

**What this changes, and why**

<!-- The *why* matters more than the *what* — the diff shows the what.
     If something was tried and ruled out, say so here; the history is
     meant to be readable by the next person debugging the same thing. -->

**How it was verified**

<!-- Which checks you ran, and what you actually observed. A test never
     seen to fail is not evidence: if you added one, say how you confirmed
     it fails when the thing it tests is broken. -->

- [ ] `make check` passes (fmt, clippy `-D warnings`, full test suite)
- [ ] `make smoke` passes, or skips (exit 77) on this machine

**Changelog**

- [ ] `CHANGELOG.md` has an entry under `## [Unreleased]`, or this change
      is not something a user would notice (refactor, test-only, CI)

**If this touches the D-Bus interface**

- [ ] `dev.marcford.BrainFM1` is only *added to* — no existing signature
      changed meaning
- [ ] `INTERFACE_VERSION` was bumped, and the changelog entry says so

**If this touches the Brain.fm API**

- [ ] The behaviour was captured from a real request, not inferred from the
      web app's bundle, and is recorded in a fixture under
      `crates/bfm-api/tests/fixtures/`
- [ ] Anything inferred rather than observed is marked as unverified in a
      doc comment next to the code that depends on it
- [ ] No test touches the live API

**Secrets check**

- [ ] Nothing in this PR — code, fixtures, logs, screenshots — contains a
      session token, a password, a signed audio URL, or a user id
