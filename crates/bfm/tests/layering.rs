//! The CLI must stay a thin client.
//!
//! If `bfm` ever gains a direct dependency on the API or audio crates, the
//! daemon has stopped being the single source of truth and every plugin built
//! against the D-Bus contract is at risk of drifting from what the CLI does.
//! Fail loudly rather than let that happen quietly.

#[test]
fn cli_does_not_depend_on_api_or_player() {
    let manifest = include_str!("../Cargo.toml");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("a [dependencies] section");

    for forbidden in ["bfm-api", "bfm-player"] {
        assert!(
            !deps.contains(forbidden),
            "bfm must not depend on {forbidden}: the CLI is a bfm-proto client only. \
             See docs/architecture.md."
        );
    }
}
