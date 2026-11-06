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

## Verification status

`make install` is verified against a `DESTDIR` for three configurations
(both GUIs, GTK only, headless). The packaging manifests in `packaging/`
are **not** built here: this machine has `makepkg` but no `flatpak-builder`,
`snapcraft`, `appimagetool`, `dpkg-deb` or `rpmbuild`. They are built in CI,
and should be treated as unproven until that pipeline has run green.
