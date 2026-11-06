# Shipping bfm to every distro

The goal is that a Linux user on any distro can install this and have it
work. That is mostly *not* achieved by publishing four bundle formats — it
is achieved by being trivially packageable, and then meeting the remaining
users where they are.

## The single most valuable thing is already done

`make && make install` with `PREFIX` and `DESTDIR` support is what every
distro packager actually needs. A PKGBUILD, a `debian/rules`, or an RPM
`%install` section each reduce to one line calling it. Detection means the
same invocation works on a headless server and a KDE desktop.

Upstreams that are hard to package get packaged late, badly, or never.
Upstreams that are easy to package get picked up by distro maintainers who
know their distro far better than we do.

## One repository, not several

Keeping packaging in branches or sibling repositories is a maintenance trap:
the version in `debian/changelog` drifts from `Cargo.toml`, CI is duplicated
per branch, and a security fix has to be landed N times. Everything lives in
`packaging/`, and CI builds artifacts from a tag.

## Format by format, honestly

| Format | Fit | Why |
|---|---|---|
| **Native (AUR, deb, rpm)** | **Best** | This is a daemon + CLI + GUI with a systemd user unit and a D-Bus service file. That is precisely what distro packaging exists to install correctly |
| **Flatpak** | Good, with a caveat | Must ship the daemon *inside* the sandbox rather than talking to a host one; a sandboxed GUI reaching a host D-Bus service is fragile. The CLI then is not on the host `PATH` |
| **Snap** | Marginal | Same sandbox questions as Flatpak, a smaller audience off Ubuntu, and strict confinement plus Secret Service for the keyring is genuinely painful |
| **AppImage** | **Poor** | AppImage is for self-contained GUI apps with no system integration. We need a systemd unit and a D-Bus activation file *installed on the host*, and D-Bus activation is how the GUI starts the daemon at all. An AppImage would ship a GUI that cannot start its own backend |

### Recommended order

1. **AUR** — the maintainer runs Arch, and `makepkg` is the one packaging
   tool available on this machine to actually test with.
2. **deb and rpm** — covers Debian/Ubuntu and Fedora/openSUSE, which is the
   large majority of remaining desktop users.
3. **Flatpak** — for users who prefer it, shipped self-contained.
4. **Snap / AppImage** — only on request. Neither suits a daemon-centric
   app, and shipping a format that works badly is worse than not shipping
   it.

## Three packages, not one

`bfm` carries the daemon, the CLI and the service files. `bfm-gtk` and
`bfm-kde` carry one binary and one desktop entry each and depend on `bfm`.

The split matters: a server should be able to install the daemon without
dragging a desktop stack onto it, and a GNOME user should not install Qt.
Distro conventions expect it too.

Note what is *not* done here: packaging the cargo artifact directly. That
would ship `bfmd` without its D-Bus service file, and the daemon would
never activate — the binary would be present and the app still broken.
Packages are built from the same staged `make install` tree the Makefile
produces, so they cannot drift from a source install.

## Verification status

Verified on this machine:

- `make install` against a `DESTDIR`, for three configurations — both GUIs,
  GTK only, and headless.
- **deb and rpm built with `nfpm` and their contents inspected.** The
  daemon package holds exactly six files, carries both service files, and
  contains no GUI binary; `bfm-gtk` holds only its binary and its desktop
  entry and declares `Depends: bfm (= 0.1.0)`.
- The `PKGBUILD` parses under `makepkg` and generates a valid `.SRCINFO`.

Not verified here:

- The **Flatpak** manifest. This machine has no `flatpak-builder`; treat it
  as unproven until CI builds it.
- Installing the packages on their actual target distros. CI installs the
  `.deb` on Ubuntu and checks the binaries land on `PATH`; the `.rpm` is
  only inspected, not installed.

### A trap worth recording

The first packages built here were **72 MB** for a daemon and a CLI,
because they were staged from a debug build. Release binaries bring the
same package to **6.9 MB**. `make package` now refuses to run unless
`PROFILE=release`, rather than relying on anyone remembering.
