//! Session token handling.
//!
//! The Brain.fm session token is an RS256 JWT whose payload carries exactly
//! three claims: `_id`, `iat`, `exp`. We decode the payload to learn the user
//! id (needed to build `/users/{uid}/…` paths) and the expiry. We deliberately
//! do **not** verify the signature: we have no key, and the server is the only
//! party whose opinion of validity matters.

use base64::Engine as _;
use serde::Deserialize;
use std::{
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{ApiError, Result};

/// The three claims a Brain.fm session token actually carries.
#[derive(Debug, Clone, Deserialize)]
pub struct Claims {
    /// Brain.fm user id — a 21-character nanoid, not a Mongo ObjectId.
    #[serde(rename = "_id")]
    pub id: String,
    pub iat: i64,
    pub exp: i64,
}

/// A session token plus its decoded claims.
///
/// `Debug` is redacted: the raw token is a bearer credential.
#[derive(Clone)]
pub struct SessionToken {
    raw: String,
    claims: Claims,
}

impl SessionToken {
    pub fn parse(raw: impl Into<String>) -> Result<Self> {
        let raw = raw.into();
        let payload = raw
            .split('.')
            .nth(1)
            .ok_or_else(|| ApiError::BadTokenEncoding("not a three-part JWT".into()))?;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|e| ApiError::BadTokenEncoding(e.to_string()))?;
        let claims: Claims = serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::BadTokenEncoding(e.to_string()))?;
        Ok(Self { raw, claims })
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    pub fn user_id(&self) -> &str {
        &self.claims.id
    }

    pub fn claims(&self) -> &Claims {
        &self.claims
    }

    /// Observed TTL is exactly 15 minutes.
    pub fn lifetime(&self) -> Duration {
        Duration::from_secs((self.claims.exp - self.claims.iat).max(0) as u64)
    }

    pub fn is_expired(&self) -> bool {
        self.expired_at(SystemTime::now())
    }

    pub fn expired_at(&self, now: SystemTime) -> bool {
        let secs = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
        secs >= self.claims.exp
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionToken")
            .field("user_id", &self.claims.id)
            .field("exp", &self.claims.exp)
            .field("raw", &"<redacted>")
            .finish()
    }
}

impl Drop for SessionToken {
    fn drop(&mut self) {
        self.raw.zeroize();
    }
}

/// A password, held only long enough to post it. Never stored, never logged.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Password(String);

impl Password {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unsigned stand-in: we never verify signatures, only decode the payload.
    fn token_with(id: &str, iat: i64, exp: i64) -> String {
        let b64 = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        format!(
            "{}.{}.{}",
            b64(r#"{"alg":"RS256","typ":"JWT"}"#),
            b64(&format!(r#"{{"_id":"{id}","iat":{iat},"exp":{exp}}}"#)),
            "c2lnbmF0dXJl"
        )
    }

    #[test]
    fn decodes_the_three_claims() {
        let t = SessionToken::parse(token_with("PLACEHOLDERUSERID0001", 1000, 1900)).unwrap();
        assert_eq!(t.user_id(), "PLACEHOLDERUSERID0001");
        assert_eq!(t.user_id().len(), 21, "user id is a 21-char nanoid");
        assert_eq!(t.lifetime(), Duration::from_secs(900), "15 minute TTL");
    }

    #[test]
    fn expiry_is_evaluated_against_a_supplied_clock() {
        let t = SessionToken::parse(token_with("u", 1000, 1900)).unwrap();
        assert!(!t.expired_at(UNIX_EPOCH + Duration::from_secs(1899)));
        assert!(t.expired_at(UNIX_EPOCH + Duration::from_secs(1900)));
    }

    #[test]
    fn debug_never_leaks_the_raw_token() {
        let t = SessionToken::parse(token_with("u", 1000, 1900)).unwrap();
        let s = format!("{t:?}");
        assert!(s.contains("<redacted>"));
        assert!(!s.contains("c2lnbmF0dXJl"));
    }

    #[test]
    fn rejects_a_non_jwt() {
        assert!(SessionToken::parse("nonsense").is_err());
    }
}

/// How to obtain a session.
///
/// Brain.fm supports four sign-in providers plus magic links, but a CLI can
/// only drive some of them directly — see `docs/auth.md` for why the SSO
/// providers are absent. Every variant here ends in the same place: a
/// [`SessionToken`].
#[derive(Debug)]
pub enum AuthMethod {
    /// `POST /v2/auth/email-login`. The only fully self-service method.
    Email { email: String, password: Password },

    /// `GET /v2/auth/magic-link/{token}`. Consume-only: nothing can request a
    /// link, so this is for when the user already has one.
    MagicLink { token: String },

    /// A session token lifted from an existing browser login.
    ///
    /// This is how SSO accounts (Google, Apple, Facebook) are supported: the
    /// provider login happens in a real browser against Brain.fm's own web
    /// app, and we adopt the resulting session. Because renewal is lazy and
    /// non-consuming, one handoff sustains the session indefinitely.
    ExistingToken { raw: String },
}

impl AuthMethod {
    /// Extract the token from a magic-link URL, or accept a bare token.
    ///
    /// Accepts the full link the user was emailed as well as just its last
    /// path segment, because people paste both.
    pub fn magic_link(input: &str) -> Result<Self> {
        let input = input.trim();
        let token = input
            .rsplit('/')
            .find(|seg| !seg.is_empty())
            .unwrap_or(input)
            // tolerate a trailing query or fragment
            .split(['?', '#'])
            .next()
            .unwrap_or("");

        if token.is_empty() {
            return Err(ApiError::BadTokenEncoding(
                "no token found in the magic link".into(),
            ));
        }
        Ok(Self::MagicLink {
            token: token.to_owned(),
        })
    }

    /// Human-readable name, for logs and error messages. Never includes
    /// credentials.
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Email { .. } => "email and password",
            Self::MagicLink { .. } => "magic link",
            Self::ExistingToken { .. } => "existing browser session",
        }
    }
}

#[cfg(test)]
mod auth_method_tests {
    use super::*;

    #[test]
    fn magic_link_accepts_a_full_url() {
        let m = AuthMethod::magic_link("https://my.brain.fm/auth/magic-link/abc123").unwrap();
        assert!(matches!(m, AuthMethod::MagicLink { ref token } if token == "abc123"));
    }

    #[test]
    fn magic_link_accepts_a_bare_token() {
        let m = AuthMethod::magic_link("abc123").unwrap();
        assert!(matches!(m, AuthMethod::MagicLink { ref token } if token == "abc123"));
    }

    #[test]
    fn magic_link_tolerates_trailing_slash_and_query() {
        for input in [
            "https://my.brain.fm/auth/magic-link/abc123/",
            "https://my.brain.fm/auth/magic-link/abc123?utm_source=email",
        ] {
            let m = AuthMethod::magic_link(input).unwrap();
            assert!(
                matches!(m, AuthMethod::MagicLink { ref token } if token == "abc123"),
                "{input}"
            );
        }
    }

    #[test]
    fn magic_link_rejects_empty_input() {
        assert!(AuthMethod::magic_link("   ").is_err());
    }

    #[test]
    fn describe_never_leaks_credentials() {
        let m = AuthMethod::Email {
            email: "a@b.c".into(),
            password: Password::new("hunter2"),
        };
        assert_eq!(m.describe(), "email and password");
        assert!(!format!("{:?}", m).contains("hunter2"));
    }
}
