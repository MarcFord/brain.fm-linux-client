//! `bfm capabilities` is a contract, so it is tested like one.
//!
//! Its whole purpose is to let a client gate on evidence instead of on a
//! version string. That only works if two things hold:
//!
//!   * it answers with no session bus and no daemon, because a client asking
//!     "what can you do" must not have to start a service to find out, and
//!   * the feature names never change, because anything gating on a name
//!     breaks silently when the name moves.
//!
//! The name list below is deliberately hard-coded rather than read from the
//! binary. A test that asks the program what it promises and then agrees with
//! it cannot catch a rename.

use std::process::Command;

/// Every name released so far. Add to this; do not edit or remove.
const RELEASED_NAMES: &[&str] = &[
    "status-unavailable-in-band",
    "watch-follows-daemon",
    "list-session-lengths",
];

/// Run `bfm capabilities` with no session bus reachable at all.
fn probe_without_a_bus(args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bfm"));
    cmd.arg("capabilities")
        .args(args)
        // Both, because unsetting the address alone still lets zbus find a
        // bus through the runtime directory.
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env("XDG_RUNTIME_DIR", "/nonexistent");
    cmd.output().expect("failed to run bfm")
}

#[test]
fn it_answers_with_no_session_bus() {
    let out = probe_without_a_bus(&["--json"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "a capability probe must not need a bus.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        !stdout.trim().is_empty(),
        "it exited cleanly but said nothing"
    );
}

#[test]
fn the_json_shape_is_what_clients_parse() {
    let out = probe_without_a_bus(&["--json"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("not JSON: {e}\n{stdout}"));

    assert!(
        v["version"].as_str().is_some_and(|s| !s.is_empty()),
        "version must be a non-empty string: {stdout}"
    );
    assert!(
        v["interfaceVersion"].as_u64().is_some(),
        "interfaceVersion must be a number: {stdout}"
    );

    let features: Vec<&str> = v["features"]
        .as_array()
        .expect("features must be an array")
        .iter()
        .map(|f| f.as_str().expect("each feature is a string"))
        .collect();

    for name in RELEASED_NAMES {
        assert!(
            features.contains(name),
            "{name:?} was released and must still be advertised; got {features:?}"
        );
    }
}

/// The plain form exists so a shell script can grep it without a JSON parser.
#[test]
fn the_plain_form_is_greppable() {
    let out = probe_without_a_bus(&[]);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(out.status.success(), "{stdout}");
    assert!(
        stdout.lines().any(|l| l.starts_with("version ")),
        "expected a `version` line: {stdout}"
    );
    assert!(
        stdout.lines().any(|l| l.starts_with("interface ")),
        "expected an `interface` line: {stdout}"
    );
    for name in RELEASED_NAMES {
        assert!(
            stdout.lines().any(|l| l == format!("feature {name}")),
            "expected `feature {name}`: {stdout}"
        );
    }
}

/// A duplicate name would make a client's gate ambiguous.
#[test]
fn names_are_unique() {
    let out = probe_without_a_bus(&["--json"]);
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    let mut names: Vec<String> = v["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_owned())
        .collect();
    let before = names.len();
    names.sort();
    names.dedup();
    assert_eq!(before, names.len(), "duplicate capability name: {names:?}");
}
