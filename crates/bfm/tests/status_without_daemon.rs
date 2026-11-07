//! `bfm status` must not report a dead daemon as a healthy `idle`.
//!
//! Every property read in `print_status` falls back to a default on error,
//! so before this was fixed a completely broken install printed `idle` and
//! exited 0 — indistinguishable from a working daemon with nothing playing,
//! which is the worst possible answer for the command people run to find out
//! whether the thing works.
//!
//! The daemon is isolated away by pointing **both** XDG data variables at a
//! nonexistent directory. `XDG_DATA_DIRS` alone is not enough: D-Bus searches
//! the home data dir independently, so with bfm actually installed to
//! `~/.local` the private bus would activate the real daemon and the test
//! would pass vacuously.

use std::process::{Command, Output};

/// Run `bfm <args>` on a private bus that has no bfm service file at all.
fn bfm_with_no_daemon(args: &[&str]) -> Option<Output> {
    if Command::new("dbus-run-session")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skip: no dbus-run-session");
        return None;
    }

    let mut cmd = Command::new("dbus-run-session");
    cmd.arg("--")
        .arg(env!("CARGO_BIN_EXE_bfm"))
        .args(args)
        .env("XDG_DATA_HOME", "/nonexistent")
        .env("XDG_DATA_DIRS", "/nonexistent")
        .env("BFM_TOKEN_STORE", "memory");
    Some(cmd.output().expect("failed to spawn dbus-run-session"))
}

#[test]
fn plain_status_fails_when_no_daemon_answers() {
    let Some(out) = bfm_with_no_daemon(&["status"]) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "expected a non-zero exit with no daemon, got success.\nstdout: {stdout}\nstderr: {stderr}"
    );
    // The regression itself: `idle` on stdout would mean a broken install
    // looks healthy.
    assert!(
        !stdout.contains("idle"),
        "reported `idle` with no daemon running; stdout: {stdout}"
    );
    // Nothing at all on stdout, so a script reading it cannot mistake the
    // outage for a state.
    assert!(
        stdout.trim().is_empty(),
        "expected empty stdout so scripts cannot read an outage as a state; got: {stdout}"
    );
    assert!(
        stderr.contains("no bfm daemon answered"),
        "expected a diagnosable message on stderr; got: {stderr}"
    );
}

/// waybar is the opposite case: if the process exits the module renders
/// nothing, so the outage has to arrive in-band with a zero exit.
#[test]
fn json_status_reports_unavailable_in_band() {
    let Some(out) = bfm_with_no_daemon(&["status", "--json"]) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        out.status.success(),
        "--json must not fail, or waybar shows nothing; stdout: {stdout}"
    );

    let line = stdout
        .lines()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or_else(|| panic!("no JSON object on stdout; got: {stdout}"));
    let v: serde_json::Value = serde_json::from_str(line).expect("stdout was not valid JSON");

    assert_eq!(
        v["class"], "unavailable",
        "waybar keys styling off `class`, so the outage must be distinguishable from idle: {line}"
    );
    assert_eq!(v["alt"], "unavailable", "{line}");
    assert_ne!(
        v["class"], "idle",
        "an absent daemon must never present as idle: {line}"
    );
}
