//! Integration tests for `bfm-api`, against recorded fixtures.
//!
//! These never touch the live API — CI enforces that. The fixtures in
//! `tests/fixtures/` are the executable form of `docs/api.md`: if Brain.fm
//! changes a shape, the fix is to re-capture, update the doc, and update these
//! together.

use bfm_api::{ApiError, AuthMethod, Client, EffectLevel, Password, SessionRequest};
use wiremock::matchers::{body_json_string, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

/// A client pointed at the mock server, with a parseable token.
fn client(server: &MockServer) -> Client {
    let base = server.uri();
    Client::new()
        .unwrap()
        .with_base_urls(format!("{base}/v2"), format!("{base}/v3"))
        .with_token(test_token(4_102_444_800)) // far future
}

/// Build an unsigned JWT. We never verify signatures, only decode the payload.
fn test_token(exp: i64) -> bfm_api::SessionToken {
    use base64::Engine as _;
    let b64 = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
    let raw = format!(
        "{}.{}.{}",
        b64(r#"{"alg":"RS256","typ":"JWT"}"#),
        b64(&format!(
            r#"{{"_id":"PLACEHOLDERUSERID0001","iat":1000,"exp":{exp}}}"#
        )),
        "c2ln"
    );
    bfm_api::SessionToken::parse(raw).unwrap()
}

fn json(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

// ---------------------------------------------------------------- headers

/// The API rejects requests that don't look like the web app. If these headers
/// ever stop being sent, every call 403s and the cause is non-obvious.
#[tokio::test]
async fn sends_browser_shaped_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v3/mentalStates/dynamic"))
        .and(header("origin", "https://my.brain.fm"))
        .and(header("referer", "https://my.brain.fm/"))
        .respond_with(json(fixture("mental_states.json")))
        .expect(1)
        .mount(&server)
        .await;

    client(&server).mental_states().await.unwrap();
}

// ---------------------------------------------------------------- catalog

#[tokio::test]
async fn parses_mental_states() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v3/mentalStates/dynamic"))
        .respond_with(json(fixture("mental_states.json")))
        .mount(&server)
        .await;

    let (states, rotation) = client(&server).mental_states().await.unwrap();

    assert_eq!(states.len(), 2);
    assert_eq!(states[0].id, "focus", "ids are slugs, not opaque");
    assert_eq!(states[0].display_value, "Focus");
    assert_eq!(
        states[0].default_activity.as_ref().unwrap().id,
        "QYN78dJrrzs5mD_TJgBcq"
    );
    assert_eq!(states[0].default_session_lengths[0].length_in_seconds, 3600);
    assert!(!rotation.is_some(), "a valid token is not rotated");
}

#[tokio::test]
async fn parses_activities() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v3/mentalStates/dynamic/focus/activities"))
        .respond_with(json(fixture("activities_focus.json")))
        .mount(&server)
        .await;

    let (acts, _) = client(&server).activities("focus").await.unwrap();
    assert_eq!(acts.len(), 5);
    assert!(
        acts.iter()
            .any(|a| a.is_default && a.display_value == "Deep Work")
    );
    assert!(acts.iter().any(|a| a.has_fixed_genres));
}

/// Genres come back as two separate pickers, not one flat list.
#[tokio::test]
async fn parses_genres_as_two_lists() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v3/mentalStates/dynamic/focus/genres"))
        .respond_with(json(fixture("genres_focus.json")))
        .mount(&server)
        .await;

    let (g, _) = client(&server).genres("focus").await.unwrap();
    assert_eq!(g.base_genre_names.len(), 10);
    assert_eq!(g.nature_genre_names.len(), 10);
    assert!(g.base_genre_names.contains(&"Electronic".to_string()));
    assert!(g.nature_genre_names.contains(&"Rain".to_string()));
}

// ---------------------------------------------------------------- playback

#[tokio::test]
async fn new_serving_yields_a_playable_url_and_ignores_the_bloat() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v3/servings/new"))
        .and(query_param("dynamicMentalStateId", "focus"))
        .respond_with(json(fixture("servings_new.json")))
        .mount(&server)
        .await;

    let (servings, _) = client(&server).new_serving("focus").await.unwrap();
    let s = &servings[0];

    assert_eq!(s.track.name, "Alcatraz");
    assert_eq!(
        s.track_variation.length_in_seconds, 1800.0,
        "30-minute tracks"
    );
    assert_eq!(s.track_variation.neural_effect_level, Some(0.06));
    assert!(
        s.track_variation.tokened_url.contains("audio2.brain.fm"),
        "tokenedUrl is the only usable URL"
    );
    assert!(s.track_variation.tokened_url.contains("expiration="));
}

/// A signed URL is a bearer credential for the audio. It must never reach a
/// log line intact.
#[tokio::test]
async fn signed_urls_are_scrubbed_for_logging() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v3/servings/new"))
        .respond_with(json(fixture("servings_new.json")))
        .mount(&server)
        .await;

    let (servings, _) = client(&server).new_serving("focus").await.unwrap();
    let logged = servings[0].track_variation.loggable_url();

    assert_eq!(logged, "https://audio2.brain.fm/alcatraz-high.mp3?<signed>");
    assert!(!logged.contains("token="));
    assert!(!logged.contains("PLACEHOLDER_SIGNATURE"));
}

#[tokio::test]
async fn session_request_serialises_the_shape_the_api_expects() {
    let server = MockServer::start().await;
    let expected = serde_json::json!({
        "dynamicActivityId": "QYN78dJrrzs5mD_TJgBcq",
        "version": 3,
        "neuralEffectLevels": ["high"],
        "genreNames": ["Electronic"]
    })
    .to_string();

    Mock::given(method("POST"))
        .and(path("/v3/users/PLACEHOLDERUSERID0001/sessions"))
        .and(query_param("platform", "web"))
        .and(body_json_string(expected))
        .respond_with(json(fixture("session_create.json")))
        .expect(1)
        .mount(&server)
        .await;

    let req = SessionRequest::new("QYN78dJrrzs5mD_TJgBcq", EffectLevel::High)
        .with_genres(["Electronic".to_string()]);
    client(&server).start_session(&req).await.unwrap();
}

/// `POST /sessions` wraps its servings in an object and returns the whole
/// queue; `GET /servings/new` returns a bare array. Modelling them the same
/// way is what made the first live `bfm play` fail.
#[tokio::test]
async fn session_create_returns_a_whole_queue_not_one_track() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v3/users/PLACEHOLDERUSERID0001/sessions"))
        .respond_with(json(fixture("session_create.json")))
        .mount(&server)
        .await;

    let req = SessionRequest::new("QYN78dJrrzs5mD_TJgBcq", EffectLevel::High);
    let (servings, _) = client(&server).start_session(&req).await.unwrap();

    assert_eq!(
        servings.len(),
        2,
        "the full queue, not just the first track"
    );
    assert_eq!(servings[0].track.name, "Alcatraz");
    assert_eq!(servings[1].track.name, "Second Track");
    assert_eq!(servings[0].track_variation.length_in_seconds, 1800.0);
}

/// The two endpoints genuinely differ, so the array shape must not parse as
/// a session response and vice versa.
#[tokio::test]
async fn the_two_track_endpoints_have_different_envelopes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v3/servings/new"))
        .respond_with(json(fixture("servings_new.json")))
        .mount(&server)
        .await;

    let (bare, _) = client(&server).new_serving("focus").await.unwrap();
    assert_eq!(bare.len(), 1, "servings/new is a bare array");
}

// ---------------------------------------------------------------- renewal

/// The whole auth design rests on this: an expired token still returns 200,
/// and the envelope carries a replacement.
#[tokio::test]
async fn expired_token_response_surfaces_a_rotated_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/users/me"))
        .respond_with(json(
            r#"{"status":200,"result":{},"token":"replacement.jwt.here"}"#.to_string(),
        ))
        .mount(&server)
        .await;

    let rotation = client(&server).renew().await.unwrap();
    assert_eq!(rotation.0.as_deref(), Some("replacement.jwt.here"));
}

#[tokio::test]
async fn valid_token_response_carries_no_rotation() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/users/me"))
        .respond_with(json(fixture("users_me.json")))
        .mount(&server)
        .await;

    assert!(!client(&server).renew().await.unwrap().is_some());
}

// ---------------------------------------------------------------- errors

/// Errors use `{status, messages}`, successes use `{status, result}`.
/// Confusing the two turns "wrong password" into "the API changed".
#[tokio::test]
async fn error_envelope_becomes_a_rejection_with_messages() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/auth/email-login"))
        .respond_with(
            ResponseTemplate::new(422).set_body_raw(fixture("error_422.json"), "application/json"),
        )
        .mount(&server)
        .await;

    let err = client(&server)
        .login(AuthMethod::Email {
            email: "user@example.invalid".into(),
            password: Password::new("wrong"),
        })
        .await
        .unwrap_err();

    match &err {
        ApiError::Rejected { status, messages } => {
            assert_eq!(*status, 422);
            assert_eq!(messages, &vec!["Invalid email or password".to_string()]);
        }
        other => panic!("expected Rejected, got {other:?}"),
    }
    assert!(
        err.needs_login(),
        "a 422 on login means the user must authenticate again"
    );
}

/// 429 arrives with an empty, non-JSON body. That must not be mistaken for a
/// schema change.
#[tokio::test]
async fn rate_limit_is_classified_not_mistaken_for_schema_drift() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/auth/email-login"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "30"))
        .mount(&server)
        .await;

    let err = client(&server)
        .login(AuthMethod::Email {
            email: "user@example.invalid".into(),
            password: Password::new("x"),
        })
        .await
        .unwrap_err();

    match err {
        ApiError::RateLimited { retry_after_secs } => assert_eq!(retry_after_secs, Some(30)),
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

#[tokio::test]
async fn malformed_token_is_400_not_401() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v3/mentalStates/dynamic"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&server)
        .await;

    let err = client(&server).mental_states().await.unwrap_err();
    assert!(matches!(err, ApiError::MalformedToken), "got {err:?}");
    assert!(err.needs_login());
}

#[tokio::test]
async fn unauthorized_asks_for_a_new_login() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v3/mentalStates/dynamic"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let err = client(&server).mental_states().await.unwrap_err();
    assert!(matches!(err, ApiError::Unauthenticated), "got {err:?}");
    assert!(err.needs_login());
}

/// When Brain.fm changes a shape, the error must say so rather than surfacing
/// a bare serde message.
#[tokio::test]
async fn unrecognised_shape_reports_a_schema_change_with_context() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v3/mentalStates/dynamic"))
        .respond_with(json(
            r#"{"status":200,"result":"not an array"}"#.to_string(),
        ))
        .mount(&server)
        .await;

    let err = client(&server).mental_states().await.unwrap_err();
    match err {
        ApiError::SchemaChanged { context, .. } => assert_eq!(context, "mentalStates"),
        other => panic!("expected SchemaChanged, got {other:?}"),
    }
    assert!(
        err.to_string().contains("likely changed"),
        "the message must point at the API, not at serde: {err}"
    );
    assert!(!err.needs_login(), "a schema change is not an auth problem");
}

// ---------------------------------------------------------------- auth

#[tokio::test]
async fn email_login_accepts_a_token_at_the_envelope_root() {
    let server = MockServer::start().await;
    let tok = test_token(4_102_444_800);
    Mock::given(method("POST"))
        .and(path("/v2/auth/email-login"))
        .and(body_json_string(
            serde_json::json!({ "email": "user@example.invalid", "password": "correct" })
                .to_string(),
        ))
        .respond_with(json(format!(
            r#"{{"status":200,"result":{{}},"token":"{}"}}"#,
            tok.as_str()
        )))
        .expect(1)
        .mount(&server)
        .await;

    let got = client(&server)
        .login(AuthMethod::Email {
            email: "user@example.invalid".into(),
            password: Password::new("correct"),
        })
        .await
        .unwrap();

    assert_eq!(got.user_id(), "PLACEHOLDERUSERID0001");
}

/// The success shape is unverified upstream, so both plausible nestings are
/// accepted. See `docs/auth.md`.
#[tokio::test]
async fn email_login_also_accepts_a_token_nested_under_result() {
    let server = MockServer::start().await;
    let tok = test_token(4_102_444_800);
    Mock::given(method("POST"))
        .and(path("/v2/auth/email-login"))
        .respond_with(json(format!(
            r#"{{"status":200,"result":{{"token":"{}","type":"email"}}}}"#,
            tok.as_str()
        )))
        .mount(&server)
        .await;

    let got = client(&server)
        .login(AuthMethod::Email {
            email: "user@example.invalid".into(),
            password: Password::new("correct"),
        })
        .await
        .unwrap();

    assert_eq!(got.user_id(), "PLACEHOLDERUSERID0001");
}

#[tokio::test]
async fn magic_link_is_consumed_with_a_get() {
    let server = MockServer::start().await;
    let tok = test_token(4_102_444_800);
    Mock::given(method("GET"))
        .and(path("/v2/auth/magic-link/abc123"))
        .respond_with(json(format!(
            r#"{{"status":200,"result":{{"token":"{}"}}}}"#,
            tok.as_str()
        )))
        .expect(1)
        .mount(&server)
        .await;

    let got = client(&server)
        .login(AuthMethod::magic_link("https://my.brain.fm/auth/magic-link/abc123").unwrap())
        .await
        .unwrap();

    assert_eq!(got.user_id(), "PLACEHOLDERUSERID0001");
}

/// An adopted browser session makes no network call — it is already a session.
#[tokio::test]
async fn existing_token_is_adopted_without_a_request() {
    let server = MockServer::start().await;
    let raw = test_token(4_102_444_800).as_str().to_owned();

    let got = client(&server)
        .login(AuthMethod::ExistingToken { raw })
        .await
        .unwrap();

    assert_eq!(got.user_id(), "PLACEHOLDERUSERID0001");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "adopting a browser session must not call the API"
    );
}

// ---------------------------------------------------------------- reactions

/// Favouriting carries the id in the path and no body; the verb is the
/// whole difference between adding and removing.
#[tokio::test]
async fn favouriting_posts_and_unfavouriting_deletes() {
    let server = MockServer::start().await;
    let path_ = "/v3/users/PLACEHOLDERUSERID0001/tracks/trk_one/favorites";

    Mock::given(method("POST"))
        .and(path(path_))
        .respond_with(json(r#"{"status":200,"result":{}}"#.to_string()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(path_))
        .respond_with(json(r#"{"status":200,"result":{}}"#.to_string()))
        .expect(1)
        .mount(&server)
        .await;

    let c = client(&server);
    c.set_favorite("trk_one", true).await.unwrap();
    c.set_favorite("trk_one", false).await.unwrap();
}

/// Dislikes are the odd one out: /v2, a query version, and a body.
#[tokio::test]
async fn disliking_posts_the_track_id_as_a_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/users/PLACEHOLDERUSERID0001/dislikes"))
        .and(query_param("version", "3"))
        .and(body_json_string(
            serde_json::json!({ "trackId": "trk_one" }).to_string(),
        ))
        .respond_with(json(r#"{"status":200,"result":{}}"#.to_string()))
        .expect(1)
        .mount(&server)
        .await;

    client(&server).dislike("trk_one").await.unwrap();
}

/// A reaction is still an API call, so it must surface a renewed token like
/// any other — otherwise a long session silently stops renewing.
#[tokio::test]
async fn a_reaction_surfaces_a_rotated_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v3/users/PLACEHOLDERUSERID0001/tracks/t/favorites"))
        .respond_with(json(
            r#"{"status":200,"result":{},"token":"fresh.jwt.here"}"#.to_string(),
        ))
        .mount(&server)
        .await;

    let rot = client(&server).set_favorite("t", true).await.unwrap();
    assert_eq!(rot.0.as_deref(), Some("fresh.jwt.here"));
}

#[tokio::test]
async fn favourites_list_parses_as_servings() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v3/users/PLACEHOLDERUSERID0001/servings/favorites"))
        .and(query_param("version", "2"))
        .respond_with(json(fixture("servings_new.json")))
        .mount(&server)
        .await;

    let (list, _) = client(&server).favorites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].track.name, "Alcatraz");
}
