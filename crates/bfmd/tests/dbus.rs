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

/// An unsigned JWT is fine here: we only ever decode the payload, never
/// verify it. Far-future expiry so nothing tries to renew.
fn adoptable_token() -> String {
    use base64::Engine as _;
    let b64 = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
    format!(
        "{}.{}.{}",
        b64(r#"{"alg":"RS256","typ":"JWT"}"#),
        b64(r#"{"_id":"PLACEHOLDERUSERID0001","iat":1000,"exp":4102444800}"#),
        "c2ln"
    )
}

/// Catalog calls must refuse before reaching the network. If this ever fails
/// it means an unauthenticated daemon is talking to api.brain.fm — which in
/// the test suite would also mean the tests are hitting the live API.
#[tokio::test]
async fn catalog_requires_auth_rather_than_calling_the_api() {
    let (conn, name) = bus_or_skip!("catalogauth");
    let p = proxy(&conn, &name).await;

    for err in [
        p.list_mental_states().await.unwrap_err(),
        p.list_activities("focus").await.unwrap_err(),
        p.list_genres("focus").await.unwrap_err(),
    ] {
        assert!(
            err.to_string().contains(bfm_proto::error_code::NEEDS_LOGIN),
            "expected needs-login before any network call, got: {err}"
        );
    }
}

/// Adopting a browser session is the route for Google/Apple/Facebook
/// accounts, and it must not touch the network.
#[tokio::test]
async fn adopting_a_token_authenticates_without_a_network_call() {
    let (conn, name) = bus_or_skip!("adopt");
    let p = proxy(&conn, &name).await;
    assert!(!p.authenticated().await.unwrap());

    p.login(bfm_proto::auth_method::TOKEN, "", &adoptable_token())
        .await
        .unwrap();

    assert!(p.authenticated().await.unwrap(), "token should be adopted");
}

#[tokio::test]
async fn logout_forgets_the_session() {
    let (conn, name) = bus_or_skip!("logout");
    let p = proxy(&conn, &name).await;

    p.login(bfm_proto::auth_method::TOKEN, "", &adoptable_token())
        .await
        .unwrap();
    assert!(p.authenticated().await.unwrap());

    p.logout().await.unwrap();
    assert!(!p.authenticated().await.unwrap());
    assert_eq!(p.state().await.unwrap(), "idle");
}

#[tokio::test]
async fn an_unknown_auth_method_is_rejected() {
    let (conn, name) = bus_or_skip!("badmethod");
    let p = proxy(&conn, &name).await;

    let err = p.login("carrier-pigeon", "", "x").await.unwrap_err();
    assert!(
        err.to_string().contains("carrier-pigeon"),
        "the error should name the bad method, got: {err}"
    );
}

#[tokio::test]
async fn a_malformed_adopted_token_is_rejected() {
    let (conn, name) = bus_or_skip!("badtoken");
    let p = proxy(&conn, &name).await;

    assert!(
        p.login(bfm_proto::auth_method::TOKEN, "", "not-a-jwt")
            .await
            .is_err()
    );
    assert!(!p.authenticated().await.unwrap());
}

/// Effort is validated before anything else, so a typo fails fast with a
/// message naming the valid values.
#[tokio::test]
async fn an_invalid_effort_is_rejected_with_the_valid_values() {
    let (conn, name) = bus_or_skip!("effort");
    let p = proxy(&conn, &name).await;
    p.login(bfm_proto::auth_method::TOKEN, "", &adoptable_token())
        .await
        .unwrap();

    let err = p
        .start_session("focus", "", &[], "maximum", "")
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("low") && msg.contains("high"), "got: {msg}");
}

// ---------------------------------------------------------------- MPRIS

/// Serve BrainFM1 *and* MPRIS, as the daemon does, so the delegation between
/// them is exercised rather than assumed.
async fn serve_with_mpris(tag: &str) -> Option<(zbus::Connection, String)> {
    let name = format!("dev.marcford.bfm.mpris{tag}");
    let conn = connection::Builder::session()
        .name(name.clone())
        .serve_at(OBJECT_PATH, BrainFm::default())
        .serve_at(bfmd::mpris::MPRIS_PATH, bfmd::mpris::Root)
        .serve_at(bfmd::mpris::MPRIS_PATH, bfmd::mpris::Player)
        .build()
        .await
        .ok()?;
    Some((conn, name))
}

macro_rules! mpris_or_skip {
    ($tag:expr) => {
        match serve_with_mpris($tag).await {
            Some(v) => v,
            None => {
                eprintln!("no session bus; skipping MPRIS test");
                return;
            }
        }
    };
}

async fn mpris_player<'a>(conn: &'a zbus::Connection, name: &str) -> zbus::Proxy<'a> {
    zbus::Proxy::new(
        conn,
        name.to_owned(),
        bfmd::mpris::MPRIS_PATH,
        "org.mpris.MediaPlayer2.Player",
    )
    .await
    .expect("mpris proxy")
}

/// Capabilities must describe what actually works. Claiming `CanSeek` would
/// make well-behaved clients offer a scrubber that silently does nothing.
#[tokio::test]
async fn mpris_capabilities_are_honest() {
    let (conn, name) = mpris_or_skip!("caps");
    let p = mpris_player(&conn, &name).await;

    assert!(p.get_property::<bool>("CanPlay").await.unwrap());
    assert!(p.get_property::<bool>("CanPause").await.unwrap());
    assert!(p.get_property::<bool>("CanGoNext").await.unwrap());
    assert!(p.get_property::<bool>("CanControl").await.unwrap());
    // Brain.fm sessions only move forward, and the decoder cannot seek.
    assert!(!p.get_property::<bool>("CanGoPrevious").await.unwrap());
    assert!(!p.get_property::<bool>("CanSeek").await.unwrap());
}

#[tokio::test]
async fn mpris_reports_a_spec_valid_playback_status() {
    let (conn, name) = mpris_or_skip!("status");
    let p = mpris_player(&conn, &name).await;

    let s: String = p.get_property("PlaybackStatus").await.unwrap();
    assert!(
        ["Playing", "Paused", "Stopped"].contains(&s.as_str()),
        "MPRIS only permits those three, got {s:?}"
    );
    assert_eq!(s, "Stopped", "a fresh daemon is stopped");
}

/// The whole point of layering MPRIS over the domain interface: one state,
/// two views of it. If these ever diverge, a media key and the CLI disagree.
#[tokio::test]
async fn mpris_and_brainfm1_share_one_state() {
    let (conn, name) = mpris_or_skip!("shared");
    let m = mpris_player(&conn, &name).await;
    let b = proxy(&conn, &name).await;

    m.set_property("Volume", 0.3f64).await.unwrap();
    assert!(
        (b.volume().await.unwrap() - 0.3).abs() < 1e-9,
        "MPRIS -> BrainFM1"
    );

    b.set_volume(0.75).await.unwrap();
    let v: f64 = m.get_property("Volume").await.unwrap();
    assert!((v - 0.75).abs() < 1e-9, "BrainFM1 -> MPRIS");
}

/// A media key on an idle player must not blow up.
#[tokio::test]
async fn mpris_playpause_is_safe_while_idle() {
    let (conn, name) = mpris_or_skip!("playpause");
    let m = mpris_player(&conn, &name).await;

    m.call_method("PlayPause", &()).await.unwrap();
    let s: String = m.get_property("PlaybackStatus").await.unwrap();
    assert_eq!(s, "Stopped");
}

#[tokio::test]
async fn mpris_metadata_is_empty_with_no_track() {
    let (conn, name) = mpris_or_skip!("meta");
    let m = mpris_player(&conn, &name).await;

    let meta: std::collections::HashMap<String, zbus::OwnedVariant> =
        m.get_property("Metadata").await.unwrap();
    assert!(
        meta.is_empty(),
        "no track means no metadata, not a fake one"
    );
}

#[tokio::test]
async fn mpris_root_identifies_itself() {
    let (conn, name) = mpris_or_skip!("root");
    let root = zbus::Proxy::new(
        &conn,
        name,
        bfmd::mpris::MPRIS_PATH,
        "org.mpris.MediaPlayer2",
    )
    .await
    .unwrap();

    assert_eq!(
        root.get_property::<String>("Identity").await.unwrap(),
        "Brain.fm"
    );
    assert!(!root.get_property::<bool>("CanQuit").await.unwrap());
    assert!(!root.get_property::<bool>("HasTrackList").await.unwrap());
}

/// Version 2 added this. A client that feature-detects on InterfaceVersion
/// must find the method actually present when the number says so.
#[tokio::test]
async fn session_lengths_exist_at_interface_version_2() {
    let (conn, name) = bus_or_skip!("lengths");
    let p = proxy(&conn, &name).await;

    assert!(
        p.interface_version().await.unwrap() >= 2,
        "ListSessionLengths requires interface version 2"
    );
    // Unauthenticated, so it must refuse rather than reach the network —
    // but refusing proves the method is served.
    let err = p.list_session_lengths("focus").await.unwrap_err();
    assert!(
        err.to_string().contains(bfm_proto::error_code::NEEDS_LOGIN),
        "expected needs-login, got: {err}"
    );
}

/// Version 4 added reactions.
#[tokio::test]
async fn reactions_exist_at_interface_version_4() {
    let (conn, name) = bus_or_skip!("reactions");
    let p = proxy(&conn, &name).await;

    assert!(p.interface_version().await.unwrap() >= 4);

    // Unauthenticated, so they must refuse before touching the network —
    // refusing proves the methods are served.
    for err in [
        p.favorite("trk", true).await.unwrap_err(),
        p.dislike("trk").await.unwrap_err(),
    ] {
        assert!(
            err.to_string().contains(bfm_proto::error_code::NEEDS_LOGIN),
            "expected needs-login before any network call, got: {err}"
        );
    }
}

/// An empty id means "whatever is playing". With nothing playing that is a
/// plain failure, not a panic and not a request for track "".
#[tokio::test]
async fn an_empty_track_id_with_nothing_playing_is_refused() {
    let (conn, name) = bus_or_skip!("emptytrack");
    let p = proxy(&conn, &name).await;
    p.login(bfm_proto::auth_method::TOKEN, "", &adoptable_token())
        .await
        .unwrap();

    let err = p.favorite("", true).await.unwrap_err();
    assert!(
        err.to_string().contains("no track"),
        "expected a clear 'no track is playing', got: {err}"
    );
}

/// Version 6 added `mode`, `spec` and `unbounded` to the `Timer` property.
///
/// A countdown and a single-cycle interval are otherwise identical on the
/// wire -- both `cycles=1, phase="work"` -- so a client could not restore what
/// the user chose, and an endless interval could not be expressed at all.
#[tokio::test]
async fn timer_mode_is_readable_at_interface_version_6() {
    let (conn, name) = bus_or_skip!("timer-mode");
    let p = proxy(&conn, &name).await;

    assert!(p.interface_version().await.unwrap() >= 6);

    // No timer set.
    p.set_timer("").await.unwrap();
    assert_eq!(
        String::try_from(p.timer().await.unwrap()["mode"].clone()).unwrap(),
        "none",
        "with no timer, mode must say so rather than leaving it to be inferred"
    );

    // A countdown and a one-cycle interval must not look the same.
    p.set_timer("50m").await.unwrap();
    let countdown = p.timer().await.unwrap();
    p.set_timer("25m/5m x1").await.unwrap();
    let interval = p.timer().await.unwrap();

    assert_eq!(
        u32::try_from(countdown["cycles"].clone()).unwrap(),
        u32::try_from(interval["cycles"].clone()).unwrap(),
        "this test is pointless unless cycles really is identical"
    );
    assert_eq!(
        String::try_from(countdown["mode"].clone()).unwrap(),
        "countdown"
    );
    assert_eq!(
        String::try_from(interval["mode"].clone()).unwrap(),
        "intervals"
    );

    // The spec comes back as set, so a front-end can restore it.
    assert_eq!(
        String::try_from(interval["spec"].clone()).unwrap(),
        "25m/5m x1"
    );

    // Endless intervals: expressible, and flagged rather than inferred from a
    // zero cycle count.
    p.set_timer("25m/5m x*").await.unwrap();
    let endless = p.timer().await.unwrap();
    assert!(
        bool::try_from(endless["unbounded"].clone()).unwrap(),
        "an endless spec must report unbounded"
    );
    assert_eq!(
        String::try_from(endless["mode"].clone()).unwrap(),
        "intervals"
    );
    assert_eq!(
        u64::try_from(endless["total"].clone()).unwrap(),
        0,
        "an endless timer has no total"
    );
}
