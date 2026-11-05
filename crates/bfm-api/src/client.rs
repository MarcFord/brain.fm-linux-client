//! HTTP client.
//!
//! Two behaviours are load-bearing and easy to get wrong:
//!
//! * **Browser-shaped headers.** The API rejects requests that don't carry the
//!   web app's `Origin` and `Referer`.
//! * **Lazy token rotation.** Never refresh ahead of time. Let the token lapse;
//!   the next response carries a replacement, which must be persisted.

use reqwest::{
    StatusCode,
    header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue, ORIGIN, REFERER, USER_AGENT},
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    API_V2, API_V3, WEB_ORIGIN,
    auth::{AuthMethod, SessionToken},
    error::{ApiError, Result},
    models::*,
};

/// A desktop Firefox UA. The API expects to be talking to a browser.
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0";

/// Returned alongside each call so the caller can persist a rotated token.
#[derive(Debug, Clone, Default)]
pub struct TokenRotation(pub Option<String>);

impl TokenRotation {
    pub fn is_some(&self) -> bool {
        self.0.is_some()
    }
}

/// Read the API's error envelope (`{status, messages}`) off a failed response.
///
/// A 429 arrives with an empty, non-JSON body, so an unparseable body here is
/// expected and must not be reported as a schema change.
async fn rejection(res: reqwest::Response) -> ApiError {
    let status = res.status().as_u16();
    let bytes = match res.bytes().await {
        Ok(b) => b,
        Err(e) => return ApiError::Transport(e),
    };
    match serde_json::from_slice::<ErrorEnvelope>(&bytes) {
        Ok(env) => ApiError::Rejected {
            status,
            messages: env.lines(),
        },
        Err(_) => ApiError::Rejected {
            status,
            messages: Vec::new(),
        },
    }
}

/// Pull a session token out of an auth response.
///
/// The success shape is unverified (our capture account is Google-linked), so
/// accept it at the envelope root or under `result`, which is where the two
/// plausible shapes put it.
fn token_from(env: &Envelope<serde_json::Value>) -> Option<String> {
    env.token.clone().or_else(|| {
        env.result
            .get("token")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    })
}

pub struct Client {
    http: reqwest::Client,
    token: Option<SessionToken>,
    v2: String,
    v3: String,
}

impl Client {
    pub fn new() -> Result<Self> {
        let mut h = HeaderMap::new();
        h.insert(ORIGIN, HeaderValue::from_static(WEB_ORIGIN));
        h.insert(REFERER, HeaderValue::from_static("https://my.brain.fm/"));
        h.insert(ACCEPT, HeaderValue::from_static("*/*"));
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        h.insert(USER_AGENT, HeaderValue::from_static(UA));

        Ok(Self {
            http: reqwest::Client::builder().default_headers(h).build()?,
            token: None,
            v2: API_V2.to_owned(),
            v3: API_V3.to_owned(),
        })
    }

    /// Point the client at different hosts.
    ///
    /// Exists so the test suite can drive a mock server. Nothing in the
    /// shipped binaries calls this.
    pub fn with_base_urls(mut self, v2: impl Into<String>, v3: impl Into<String>) -> Self {
        self.v2 = v2.into();
        self.v3 = v3.into();
        self
    }

    pub fn with_token(mut self, token: SessionToken) -> Self {
        self.token = Some(token);
        self
    }

    pub fn token(&self) -> Option<&SessionToken> {
        self.token.as_ref()
    }

    /// Adopt a rotated token. Called whenever a response envelope carries a
    /// replacement; see the lazy-renewal rule in `docs/api.md`.
    pub fn set_token(&mut self, token: SessionToken) {
        self.token = Some(token);
    }

    /// Apply a [`TokenRotation`] if the server issued one, returning the raw
    /// token so the caller can persist it.
    pub fn adopt_rotation(&mut self, rotation: &TokenRotation) -> Option<String> {
        let raw = rotation.0.clone()?;
        match SessionToken::parse(raw.clone()) {
            Ok(t) => {
                self.set_token(t);
                Some(raw)
            }
            Err(e) => {
                tracing::warn!(error = %e, "server sent a token we could not decode");
                None
            }
        }
    }

    pub fn forget_token(&mut self) {
        self.token = None;
    }

    fn user_id(&self) -> Result<&str> {
        Ok(self
            .token
            .as_ref()
            .ok_or(ApiError::Unauthenticated)?
            .user_id())
    }

    /// Issue a request and unwrap the envelope, surfacing any rotated token.
    async fn send<T: DeserializeOwned>(
        &self,
        req: reqwest::RequestBuilder,
        context: &'static str,
    ) -> Result<(T, TokenRotation)> {
        let req = match &self.token {
            Some(t) => req.bearer_auth(t.as_str()),
            None => req,
        };
        let res = req.send().await?;

        match res.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(ApiError::Unauthenticated);
            }
            // The API answers 400 for a malformed token, not 401.
            StatusCode::BAD_REQUEST => return Err(ApiError::MalformedToken),
            StatusCode::TOO_MANY_REQUESTS => {
                let retry_after_secs = res
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok());
                return Err(ApiError::RateLimited { retry_after_secs });
            }
            StatusCode::UNPROCESSABLE_ENTITY => return Err(rejection(res).await),
            s if !s.is_success() => return Err(ApiError::Http { status: s.as_u16() }),
            _ => {}
        }

        let bytes = res.bytes().await?;
        let env: Envelope<T> = serde_json::from_slice(&bytes)
            .map_err(|source| ApiError::SchemaChanged { context, source })?;

        if env.token.is_some() {
            tracing::debug!("session token was past expiry; server issued a replacement");
        }
        Ok((env.result, TokenRotation(env.token)))
    }

    // ---- catalog ----

    pub async fn mental_states(&self) -> Result<(Vec<MentalState>, TokenRotation)> {
        self.send(
            self.http.get(format!("{}/mentalStates/dynamic", self.v3)),
            "mentalStates",
        )
        .await
    }

    pub async fn activities(&self, state: &str) -> Result<(Vec<Activity>, TokenRotation)> {
        self.send(
            self.http.get(format!(
                "{}/mentalStates/dynamic/{state}/activities",
                self.v3
            )),
            "activities",
        )
        .await
    }

    pub async fn genres(&self, state: &str) -> Result<(Genres, TokenRotation)> {
        self.send(
            self.http
                .get(format!("{}/mentalStates/dynamic/{state}/genres", self.v3)),
            "genres",
        )
        .await
    }

    // ---- playback ----

    /// Fetch a playable track for a mental state.
    ///
    /// This is what the web player uses for ongoing playback, in preference to
    /// creating a session.
    pub async fn new_serving(&self, state: &str) -> Result<(Vec<Serving>, TokenRotation)> {
        self.send(
            self.http
                .get(format!("{}/servings/new", self.v3))
                .query(&[("dynamicMentalStateId", state)]),
            "servings/new",
        )
        .await
    }

    /// Start a session, receiving its whole queue.
    ///
    /// Unlike [`Client::new_serving`], this wraps the servings in an object
    /// and returns the full run of tracks, so a caller should play through
    /// the queue rather than asking for each next track.
    pub async fn start_session(
        &self,
        req: &SessionRequest,
    ) -> Result<(Vec<Serving>, TokenRotation)> {
        let uid = self.user_id()?;
        let (session, rotation): (SessionResponse, _) = self
            .send(
                self.http
                    .post(format!("{}/users/{uid}/sessions", self.v3))
                    .query(&[("platform", "web")])
                    .json(req),
                "sessions",
            )
            .await?;
        Ok((session.servings, rotation))
    }

    // ---- reactions ----

    /// Favourite or un-favourite a track.
    ///
    /// The id travels in the path and there is no body; POST adds, DELETE
    /// removes. *(Path and verbs read from the web bundle; not exercised
    /// against the live API, because doing so changes real account data.)*
    pub async fn set_favorite(&self, track_id: &str, on: bool) -> Result<TokenRotation> {
        let uid = self.user_id()?;
        let url = format!("{}/users/{uid}/tracks/{track_id}/favorites", self.v3);
        let req = if on {
            self.http.post(url)
        } else {
            self.http.delete(url)
        };
        let (_, rotation): (serde_json::Value, _) = self.send(req, "favorites").await?;
        Ok(rotation)
    }

    /// Which tracks the user has favourited.
    pub async fn favorites(&self) -> Result<(Vec<Serving>, TokenRotation)> {
        let uid = self.user_id()?;
        self.send(
            self.http
                .get(format!("{}/users/{uid}/servings/favorites", self.v3))
                .query(&[("version", "2")]),
            "favorites-list",
        )
        .await
    }

    /// Dislike a track, so it stops being served.
    ///
    /// Unlike favourites this carries a body, and it lives on `/v2`.
    /// *(Shape read from the web bundle; unverified for the same reason.)*
    pub async fn dislike(&self, track_id: &str) -> Result<TokenRotation> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            track_id: &'a str,
        }

        let uid = self.user_id()?;
        let (_, rotation): (serde_json::Value, _) = self
            .send(
                self.http
                    .post(format!("{}/users/{uid}/dislikes", self.v2))
                    .query(&[("version", "3")])
                    .json(&Body { track_id }),
                "dislikes",
            )
            .await?;
        Ok(rotation)
    }

    // ---- account ----

    pub async fn me(&self) -> Result<(serde_json::Value, TokenRotation)> {
        self.send(self.http.get(format!("{}/users/me", self.v2)), "users/me")
            .await
    }

    /// Cheapest call that will mint a replacement for an expired token.
    pub async fn renew(&self) -> Result<TokenRotation> {
        Ok(self.me().await?.1)
    }

    /// Fetch cover artwork from Brain.fm's CDN.
    ///
    /// Images only. Audio is never written to disk beyond the playback
    /// buffer — see the retention note in the README. Capped so a surprising
    /// response cannot balloon memory.
    pub async fn fetch_image(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let res = self.http.get(url).send().await?;
        if !res.status().is_success() {
            return Err(ApiError::Http {
                status: res.status().as_u16(),
            });
        }
        if let Some(len) = res.content_length()
            && len as usize > max_bytes
        {
            return Err(ApiError::Http { status: 413 });
        }
        let bytes = res.bytes().await?;
        if bytes.len() > max_bytes {
            return Err(ApiError::Http { status: 413 });
        }
        Ok(bytes.to_vec())
    }

    // ---- auth ----

    /// Obtain a session by whichever method suits the account.
    ///
    /// All providers converge here. SSO accounts arrive as
    /// [`AuthMethod::ExistingToken`] because we cannot drive Google, Apple or
    /// Facebook OAuth ourselves — see `docs/auth.md`.
    pub async fn login(&self, method: AuthMethod) -> Result<SessionToken> {
        match method {
            AuthMethod::Email { email, password } => self.email_login(&email, &password).await,
            AuthMethod::MagicLink { token } => self.consume_magic_link(&token).await,
            // Nothing to exchange: validate it parses, and let the first API
            // call be the real test of whether it is live.
            AuthMethod::ExistingToken { raw } => SessionToken::parse(raw),
        }
    }

    /// Consume a magic link.
    ///
    /// Note this is a **GET**, and that nothing can request a link — the web
    /// app only ever consumes them. *(Unverified against the live API.)*
    pub async fn consume_magic_link(&self, token: &str) -> Result<SessionToken> {
        let res = self
            .http
            .get(format!("{}/auth/magic-link/{token}", self.v2))
            .send()
            .await?;

        match res.status() {
            StatusCode::UNPROCESSABLE_ENTITY => return Err(rejection(res).await),
            StatusCode::TOO_MANY_REQUESTS => {
                return Err(ApiError::RateLimited {
                    retry_after_secs: None,
                });
            }
            StatusCode::UNAUTHORIZED | StatusCode::NOT_FOUND => {
                return Err(ApiError::Unauthenticated);
            }
            s if !s.is_success() => return Err(ApiError::Http { status: s.as_u16() }),
            _ => {}
        }

        let bytes = res.bytes().await?;
        let env: Envelope<serde_json::Value> =
            serde_json::from_slice(&bytes).map_err(|source| ApiError::SchemaChanged {
                context: "magic-link",
                source,
            })?;
        SessionToken::parse(token_from(&env).ok_or(ApiError::Unauthenticated)?)
    }

    /// Log in with email and password.
    ///
    /// NOTE: the request/response bodies are **unverified** — the capture
    /// session on 2026-11-05 authenticated with Google, so this path was never
    /// exercised against the live API. Confirm before relying on it.
    pub async fn email_login(
        &self,
        email: &str,
        password: &crate::auth::Password,
    ) -> Result<SessionToken> {
        #[derive(Serialize)]
        struct Body<'a> {
            email: &'a str,
            password: &'a str,
        }

        let res = self
            .http
            .post(format!("{}/auth/email-login", self.v2))
            .json(&Body {
                email,
                password: password.expose(),
            })
            .send()
            .await?;

        match res.status() {
            // 422 covers bad credentials, a missing field and an empty body
            // alike - the status does not distinguish them.
            StatusCode::UNPROCESSABLE_ENTITY => return Err(rejection(res).await),
            // Arrives after only a handful of rapid attempts, with an empty
            // body. Back off; do not retry in a loop.
            StatusCode::TOO_MANY_REQUESTS => {
                let retry_after_secs = res
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok());
                return Err(ApiError::RateLimited { retry_after_secs });
            }
            StatusCode::UNAUTHORIZED => return Err(ApiError::Unauthenticated),
            s if !s.is_success() => return Err(ApiError::Http { status: s.as_u16() }),
            _ => {}
        }

        let bytes = res.bytes().await?;
        let env: Envelope<serde_json::Value> =
            serde_json::from_slice(&bytes).map_err(|source| ApiError::SchemaChanged {
                context: "email-login",
                source,
            })?;

        SessionToken::parse(token_from(&env).ok_or(ApiError::Unauthenticated)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(json: &str) -> Envelope<serde_json::Value> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn token_at_envelope_root_is_found() {
        assert_eq!(
            token_from(&env(r#"{"status":200,"result":{},"token":"a.b.c"}"#)).as_deref(),
            Some("a.b.c")
        );
    }

    #[test]
    fn token_nested_under_result_is_found() {
        assert_eq!(
            token_from(&env(r#"{"status":200,"result":{"token":"a.b.c"}}"#)).as_deref(),
            Some("a.b.c")
        );
    }

    #[test]
    fn root_token_wins_over_nested() {
        let e = env(r#"{"status":200,"result":{"token":"nested"},"token":"root"}"#);
        assert_eq!(token_from(&e).as_deref(), Some("root"));
    }

    #[test]
    fn absent_token_is_none() {
        assert!(token_from(&env(r#"{"status":200,"result":{}}"#)).is_none());
    }
}
