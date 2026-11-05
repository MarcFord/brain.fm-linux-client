//! The GUI is a client like any other.

#[test]
fn gtk_front_end_does_not_depend_on_api_or_player() {
    let manifest = include_str!("../Cargo.toml");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("a [dependencies] section");

    for forbidden in ["bfm-api", "bfm-player"] {
        assert!(
            !deps.contains(forbidden),
            "bfm-gtk must not depend on {forbidden}: a front-end talks to the \
             daemon over D-Bus and has no privileges a third-party app lacks."
        );
    }
}
