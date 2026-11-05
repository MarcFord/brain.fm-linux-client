//! Front-ends are clients, not privileged insiders.
//!
//! If a UI crate ever reaches for the API or the audio engine directly, the
//! daemon has stopped being the single source of truth and the D-Bus contract
//! starts drifting from what the first-party apps actually do — which is
//! exactly the position every third-party plugin would be left in.

#[test]
fn ui_core_does_not_depend_on_api_or_player() {
    let manifest = include_str!("../Cargo.toml");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("a [dependencies] section");

    for forbidden in ["bfm-api", "bfm-player"] {
        assert!(
            !deps.contains(forbidden),
            "bfm-ui-core must not depend on {forbidden}: front-ends are \
             bfm-proto clients only. See docs/architecture.md."
        );
    }
}
