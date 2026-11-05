//! Contract tests for `dev.marcford.BrainFM1`.
//!
//! The D-Bus surface is this project's public API, so it is tested like one:
//! the real interface is served on a bus and driven through the **published
//! proxy** from `bfm-proto`, which is what a plugin author would use. Testing
//! through the proxy means a change that breaks third-party clients breaks
//! this suite too.
//!
//! Run under a private bus: `dbus-run-session -- cargo test -p bfmd`.

use std::time::Duration;

use bfm_proto::{BrainFM1Proxy, INTERFACE_VERSION, OBJECT_PATH};
use bfmd::iface::BrainFm;
use zbus::connection;

/// Serve the interface on the session bus under a unique name, so concurrent
/// tests never collide over a well-known name.
async fn serve(tag: &str) -> Option<(zbus::Connection, String)> {
    let name = format!("dev.marcford.bfm.test{tag}");
    // In zbus 6 only `build` is fallible.
    let conn = connection::Builder::session()
        .name(name.clone())
        .serve_at(OBJECT_PATH, BrainFm::default())
        .build()
        .await
        .ok()?;
    Some((conn, name))
}

/// Skips rather than fails when there is no bus, so `cargo test` on a headless
/// box stays green. CI runs the suite under `dbus-run-session`, where this
/// never trips.
macro_rules! bus_or_skip {
    ($tag:expr) => {
        match serve($tag).await {
            Some(v) => v,
            None => {
                eprintln!("no session bus; skipping D-Bus contract test");
                return;
            }
        }
    };
}

async fn proxy<'a>(conn: &'a zbus::Connection, name: &str) -> BrainFM1Proxy<'a> {
    BrainFM1Proxy::builder(conn)
        .destination(name.to_owned())
        .path(OBJECT_PATH)
        .build()
        .await
        .expect("proxy")
}

#[tokio::test]
async fn interface_version_is_discoverable() {
    let (conn, name) = bus_or_skip!("ver");
    let p = proxy(&conn, &name).await;

    assert_eq!(
        p.interface_version().await.unwrap(),
        INTERFACE_VERSION,
        "clients feature-detect on this; it must be readable"
    );
}

#[tokio::test]
async fn a_fresh_daemon_is_idle_and_unauthenticated() {
    let (conn, name) = bus_or_skip!("idle");
    let p = proxy(&conn, &name).await;

    assert_eq!(p.state().await.unwrap(), "idle");
    assert!(!p.authenticated().await.unwrap());
}

/// The timer is deliberately usable without a session or credentials, so a
/// plugin can drive one on its own.
#[tokio::test]
async fn timer_round_trips_through_the_published_proxy() {
    let (conn, name) = bus_or_skip!("timer");
    let p = proxy(&conn, &name).await;

    p.set_timer("25m/5m x4").await.unwrap();

    let t = p.timer().await.unwrap();
    let get = |k: &str| -> u64 {
        u64::try_from(t.get(k).expect(k).clone())
            .unwrap_or_else(|_| u32::try_from(t.get(k).expect(k).clone()).expect("numeric") as u64)
    };
    assert_eq!(get("cycles"), 4);
    assert_eq!(get("cycle"), 1);
    assert_eq!(get("remaining"), 25 * 60);
    // Four work periods plus the three breaks between them.
    assert_eq!(get("total"), 25 * 60 * 4 + 5 * 60 * 3);

    let phase: String = String::try_from(t.get("phase").unwrap().clone()).unwrap();
    assert_eq!(phase, "work");
}

#[tokio::test]
async fn an_unparseable_timer_is_rejected_with_a_useful_message() {
    let (conn, name) = bus_or_skip!("badtimer");
    let p = proxy(&conn, &name).await;

    let err = p.set_timer("banana").await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("banana") || msg.to_lowercase().contains("number"),
        "the error should say what was wrong, got: {msg}"
    );
}

#[tokio::test]
async fn clearing_the_timer_resets_it() {
    let (conn, name) = bus_or_skip!("cleartimer");
    let p = proxy(&conn, &name).await;

    p.set_timer("10m").await.unwrap();
    p.set_timer("").await.unwrap();

    let t = p.timer().await.unwrap();
    let phase: String = String::try_from(t.get("phase").unwrap().clone()).unwrap();
    assert_eq!(phase, "none");
}

/// Anything needing the API must say "log in", not fail obscurely. The code is
/// a stable string that clients match on.
#[tokio::test]
async fn playback_without_credentials_asks_for_login() {
    let (conn, name) = bus_or_skip!("auth");
    let p = proxy(&conn, &name).await;

    let err = p
        .start_session("focus", "", &[], "high", "")
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains(bfm_proto::error_code::NEEDS_LOGIN),
        "expected the stable needs-login code, got: {err}"
    );
}

#[tokio::test]
async fn volume_is_writable_and_clamped() {
    let (conn, name) = bus_or_skip!("vol");
    let p = proxy(&conn, &name).await;

    p.set_volume(0.25).await.unwrap();
    assert!((p.volume().await.unwrap() - 0.25).abs() < f64::EPSILON);

    p.set_volume(9.0).await.unwrap();
    assert!((p.volume().await.unwrap() - 1.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn get_status_returns_a_scriptable_snapshot() {
    let (conn, name) = bus_or_skip!("status");
    let p = proxy(&conn, &name).await;
    p.set_timer("5m").await.unwrap();

    let s = p.get_status().await.unwrap();
    for key in [
        "state",
        "authenticated",
        "volume",
        "timer_phase",
        "timer_remaining",
    ] {
        assert!(s.contains_key(key), "status is missing {key}");
    }
    let _ = Duration::from_secs(0);
}
