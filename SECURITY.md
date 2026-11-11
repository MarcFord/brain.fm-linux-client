# Security Policy

## What this project handles

`bfm` holds a Brain.fm **session token**, stored in the system keyring (or a
`0600` file where no keyring daemon exists), and presents it to
`api.brain.fm` on the user's behalf. `bfm login --browser` reads that token
out of the user's own Chromium-family browser profile. Nothing else secret
passes through: passwords are never persisted, and signed audio URLs are
scrubbed from logs.

The things most worth a report, then, are any way the client could:

- send a token, password, or other credential **anywhere other than
  Brain.fm**, including to Brain.fm when it is not Brain.fm's own credential;
- store a credential somewhere other than the keyring or the `0600`
  fallback, or write one to a log, a terminal, or a D-Bus signal;
- be made to read browser data it should not — anything outside this one
  application's `localStorage` records;
- be driven over the session bus by another process in a way that exposes
  the session rather than merely controlling playback.

See [`docs/auth.md`](docs/auth.md) for the authentication design and the
constraints behind it.

## Supported versions

Only the **latest release** receives security fixes. There are no
maintenance branches; a fix is released as a new version and noted under
`### Security` in [`CHANGELOG.md`](CHANGELOG.md).

| Version | Supported |
|---|---|
| latest release | yes |
| anything older | no — upgrade |

## Reporting a vulnerability

**Please do not open a public issue for a security problem.**

Report it privately through GitHub's vulnerability reporting for this
repository:

<https://github.com/MarcFord/brain.fm-linux-client/security/advisories/new>

If that is not an option, email the maintainer at marc.ford@gmail.com with
`[bfm security]` in the subject line.

Include what you can of:

- the affected version (`bfm --version`, `bfmd --version`) or commit;
- the file and function involved, if you have found it;
- steps to reproduce, or the shape of the input that triggers it;
- what an attacker gains — which credential, which data, which action.

**Never include a real session token, password, signed audio URL, or your
Brain.fm user id in a report.** Describe the shape instead; a redacted
example is fine.

## What to expect

- An acknowledgement within **7 days**.
- An assessment of whether it is a vulnerability, and of its severity, as
  soon as it can be reproduced. You will hear back either way.
- A fix in the next release, with a changelog entry crediting you unless
  you ask otherwise. For anything that leaks a credential, the release is
  cut as soon as the fix is verified rather than waiting on other work.
- A GitHub security advisory published alongside the release.

This is a single-maintainer project, so the timelines above are commitments
to respond, not guarantees of a fix on a fixed schedule. If you have heard
nothing after 7 days, send a follow-up; the message was lost, not ignored.

## Scope notes

- **Brain.fm's own service is out of scope.** This client has no privileged
  access to it. A problem in `api.brain.fm` or the web app belongs to
  Brain.fm, not here.
- **Reading your own browser profile is by design.** `bfm login --browser`
  reads `localStorage` from the current user's own Chromium-family profiles,
  on the user's own machine, for one origin only. A report that it reads
  *more* than that is very much in scope.
- **The session bus is a trust boundary we share with every other user
  session service.** Any process in the user's session can already talk to
  `bfmd`; that is how the CLI and GUIs work. A report that the bus exposes
  the token, or lets a caller do something the D-Bus contract does not
  advertise, is in scope. "Another process can press play" is not.
