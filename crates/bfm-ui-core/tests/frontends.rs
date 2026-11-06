//! Architecture checks that span every front-end.
//!
//! These live here rather than in each front-end because `bfm-kde` cannot
//! host a test binary: cxx-qt generates its initializer symbols for the
//! binary target, and a separate test executable fails to link against
//! them. One place covering all front-ends is better anyway — a new one
//! cannot forget to bring its own guard.

const FRONTENDS: [(&str, &str); 3] = [
    ("bfm", include_str!("../../bfm/Cargo.toml")),
    ("bfm-gtk", include_str!("../../bfm-gtk/Cargo.toml")),
    ("bfm-kde", include_str!("../../bfm-kde/Cargo.toml")),
];

/// A front-end talks to the daemon and nothing else. The moment one can
/// reach Brain.fm directly, the daemon stops being the single source of
/// truth and every third-party client becomes a second-class citizen.
#[test]
fn no_front_end_depends_on_the_api_or_the_audio_engine() {
    for (name, manifest) in FRONTENDS {
        let deps = manifest
            .split("[dependencies]")
            .nth(1)
            .unwrap_or_else(|| panic!("{name} has no [dependencies] section"));

        for forbidden in ["bfm-api", "bfm-player"] {
            assert!(
                !deps.contains(forbidden),
                "{name} must not depend on {forbidden}: front-ends are \
                 bfm-proto clients only. See docs/architecture.md."
            );
        }
    }
}

/// Both GUIs must render the same words, or they drift and nobody notices
/// until the two are compared side by side.
#[test]
fn both_guis_render_shared_display_logic() {
    let gtk = include_str!("../../bfm-gtk/src/app.rs");
    let kde = include_str!("../../bfm-kde/src/bridge.rs");

    for shared in ["page", "track_name", "timer_label"] {
        assert!(
            gtk.contains(&format!("display::{shared}")),
            "bfm-gtk should render display::{shared} from the shared core"
        );
        assert!(
            kde.contains(&format!("display::{shared}")),
            "bfm-kde should render display::{shared} from the shared core"
        );
    }
}

/// The wizard asks the same questions in the same order in both GUIs,
/// because both defer to `steps::next` rather than hard-coding a sequence.
#[test]
fn both_guis_defer_to_the_shared_step_sequencing() {
    let gtk = include_str!("../../bfm-gtk/src/wizard.rs");
    let kde = include_str!("../../bfm-kde/src/bridge.rs");

    assert!(
        gtk.contains("steps::next"),
        "bfm-gtk should use steps::next"
    );
    assert!(
        kde.contains("steps::next"),
        "bfm-kde should use steps::next"
    );
}

/// The icon name appears in five places: the file on disk, two desktop
/// entries, and a constant in each front-end. They must agree.
///
/// This is not hypothetical — both desktop entries once referenced
/// `dev.marcford.bfm` when no such file was installed, so every launcher
/// showed a generic placeholder and nothing complained.
#[test]
fn the_icon_name_agrees_everywhere() {
    const ICON: &str = "dev.marcford.bfm";

    let gtk_desktop = include_str!("../../../dist/dev.marcford.bfm.gtk.desktop");
    let kde_desktop = include_str!("../../../dist/dev.marcford.bfm.kde.desktop");
    for (name, entry) in [("gtk", gtk_desktop), ("kde", kde_desktop)] {
        let icon_line = entry
            .lines()
            .find(|l| l.starts_with("Icon="))
            .unwrap_or_else(|| panic!("{name} desktop entry has no Icon="));
        assert_eq!(
            icon_line,
            format!("Icon={ICON}"),
            "{name} desktop entry points at the wrong icon"
        );
    }

    for (name, src) in [
        ("bfm-gtk", include_str!("../../bfm-gtk/src/main.rs")),
        ("bfm-kde", include_str!("../../bfm-kde/src/main.rs")),
    ] {
        assert!(
            src.contains(&format!(r#"ICON_NAME: &str = "{ICON}""#)),
            "{name} should set the window icon to {ICON}"
        );
    }
}

/// An `Icon=` naming a file nobody installs is the failure mode above, so
/// check the artwork is actually there and is really an SVG.
#[test]
fn the_icon_file_exists_and_is_svg() {
    let svg = include_str!("../../../dist/icons/dev.marcford.bfm.svg");
    assert!(svg.contains("<svg"), "the app icon is not an SVG");
    assert!(
        svg.contains("viewBox"),
        "the icon needs a viewBox or it will not scale in a launcher"
    );

    let symbolic = include_str!("../../../dist/icons/dev.marcford.bfm-symbolic.svg");
    assert!(symbolic.contains("<svg"), "the symbolic icon is not an SVG");
}
