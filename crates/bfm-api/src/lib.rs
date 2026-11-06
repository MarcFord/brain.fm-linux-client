//! Client for Brain.fm's **private, undocumented** web API.
//!
//! Unofficial and unendorsed. The surface this models was captured from live
//! traffic and is pinned by the fixtures in `tests/fixtures/`. It can change
//! without notice; when it does, the fix belongs in this crate and nowhere else.
//!
//! Two invariants hold everything together:
//!
//! 1. **Every response is an [`Envelope`]** of `{ status, result }`, which may
//!    additionally carry a rotated `token`.
//! 2. **Renewal is lazy.** A token past its 15-minute expiry still returns `200`,
//!    and the envelope then carries a replacement. Callers never refresh
//!    proactively; they persist whatever [`Client`] hands back.

pub mod auth;
pub mod client;
pub mod error;
pub mod models;

pub use auth::{AuthMethod, Claims, Password, SessionToken};
pub use client::{Client, TokenRotation};
pub use error::{ApiError, Result};
pub use models::*;

/// Base URL for `/v2` endpoints.
pub const API_V2: &str = "https://api.brain.fm/v2";
/// Base URL for `/v3` endpoints.
pub const API_V3: &str = "https://api.brain.fm/v3";
/// The API rejects requests that do not look like the web app.
pub const WEB_ORIGIN: &str = "https://my.brain.fm";
