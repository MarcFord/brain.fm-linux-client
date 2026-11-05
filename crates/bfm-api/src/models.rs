//! Wire types.
//!
//! These mirror the JSON exactly (camelCase, Brain.fm's field names). They are
//! deliberately kept separate from any domain model the daemon uses, so a
//! schema change upstream is a one-file edit here.
//!
//! Only the fields we actually need are declared. `track` in particular ships
//! ~550 fields of denormalised `relatedActivities`; we let all of it fall on
//! the floor.

use serde::{Deserialize, Serialize};

/// Every response is `{ status, result }`, plus a `token` when the presented
/// token had expired and the server minted a replacement.
#[derive(Debug, Clone, Deserialize)]
pub struct Envelope<T> {
    pub status: u32,
    pub result: T,
    /// Present **only** when the request's token was past expiry.
    #[serde(default)]
    pub token: Option<String>,
}

/// Error responses use a **different** envelope: `messages`, not `result`.
///
/// Observed on `POST /v2/auth/email-login`, which answers `422` for bad
/// credentials, a missing field and an empty body alike. Modelling every
/// response as [`Envelope`] will fail to deserialize these.
#[derive(Debug, Clone, Deserialize)]
pub struct ErrorEnvelope {
    pub status: u32,
    #[serde(default)]
    pub messages: Vec<serde_json::Value>,
}

impl ErrorEnvelope {
    /// Human-readable lines, for surfacing to the user.
    pub fn lines(&self) -> Vec<String> {
        self.messages
            .iter()
            .map(|v| match v.as_str() {
                Some(s) => s.to_owned(),
                None => v.to_string(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MentalState {
    /// A human-readable slug: `focus`, `relax`, `sleep`, `meditate`.
    pub id: String,
    pub display_value: String,
    #[serde(default)]
    pub default_activity: Option<Activity>,
    #[serde(default)]
    pub default_session_lengths: Vec<SessionLength>,
    #[serde(default)]
    pub order: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionLength {
    pub id: String,
    pub length_in_seconds: u32,
    pub display_value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    /// A 21-character nanoid.
    pub id: String,
    pub display_value: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub has_fixed_genres: bool,
}

/// Genres come back split into two pickers, not one flat list.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Genres {
    #[serde(default)]
    pub base_genre_names: Vec<String>,
    #[serde(default)]
    pub nature_genre_names: Vec<String>,
}

/// What `POST /v3/users/{uid}/sessions` actually returns.
///
/// Note this is **not** the shape of `GET /v3/servings/new`, which returns a
/// bare array. A session wraps its servings in an object — and hands over the
/// entire queue at once (40 observed), not one track. There is therefore no
/// need to call the API again between tracks.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub servings: Vec<Serving>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Serving {
    pub track: Track,
    pub track_variation: TrackVariation,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub beats_per_minute: Option<f64>,
    #[serde(default)]
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tag {
    #[serde(rename = "type")]
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackVariation {
    pub id: String,
    /// The **only** usable audio URL. `cdnUrl` 403s unsigned; `baseUrl` and
    /// `url` are relative fragments, not URLs.
    pub tokened_url: String,
    /// Observed at 1800 (30 minutes).
    pub length_in_seconds: f64,
    /// A float in `0..=1` — note the request side uses enum strings instead.
    #[serde(default)]
    pub neural_effect_level: Option<f64>,
    #[serde(default)]
    pub style: Option<String>,
}

impl TrackVariation {
    /// Safe for logs: drops the signing parameters.
    pub fn loggable_url(&self) -> String {
        crate::error::scrub_audio_url(&self.tokened_url)
    }
}

/// Requested neural effect. Serialises to the lowercase strings the API wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EffectLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRequest {
    pub dynamic_activity_id: String,
    pub version: u8,
    pub neural_effect_levels: Vec<EffectLevel>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub genre_names: Vec<String>,
}

impl SessionRequest {
    pub fn new(activity_id: impl Into<String>, effect: EffectLevel) -> Self {
        Self {
            dynamic_activity_id: activity_id.into(),
            version: 3,
            neural_effect_levels: vec![effect],
            genre_names: Vec::new(),
        }
    }

    pub fn with_genres(mut self, genres: impl IntoIterator<Item = String>) -> Self {
        self.genre_names = genres.into_iter().collect();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_without_rotation_parses() {
        let j = r#"{"status":200,"result":[]}"#;
        let e: Envelope<Vec<MentalState>> = serde_json::from_str(j).unwrap();
        assert_eq!(e.status, 200);
        assert!(e.token.is_none(), "no rotation while the token is valid");
    }

    #[test]
    fn envelope_with_rotation_surfaces_the_new_token() {
        let j = r#"{"status":200,"result":[],"token":"a.b.c"}"#;
        let e: Envelope<Vec<MentalState>> = serde_json::from_str(j).unwrap();
        assert_eq!(e.token.as_deref(), Some("a.b.c"));
    }

    #[test]
    fn effect_level_serialises_lowercase() {
        assert_eq!(
            serde_json::to_string(&EffectLevel::High).unwrap(),
            r#""high""#
        );
    }

    #[test]
    fn session_request_omits_empty_genres() {
        let r = SessionRequest::new("QYN78dJrrzs5mD_TJgBcq", EffectLevel::High);
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains(r#""dynamicActivityId":"QYN78dJrrzs5mD_TJgBcq""#));
        assert!(j.contains(r#""version":3"#));
        assert!(!j.contains("genreNames"));
    }

    #[test]
    fn track_variation_ignores_the_denormalised_bloat() {
        let j = r#"{
            "id":"tv1","url":"rel","baseUrl":"rel","cdnUrl":"https://audio2.brain.fm/x.mp3",
            "tokenedUrl":"https://audio2.brain.fm/x.mp3?expiration=1801687490428&token=abc",
            "lengthInSeconds":1800,"neuralEffectLevel":0.06,"style":"normal",
            "somethingBrandNew":{"we":"have not seen this"}
        }"#;
        let tv: TrackVariation = serde_json::from_str(j).unwrap();
        assert_eq!(tv.length_in_seconds, 1800.0);
        assert_eq!(tv.loggable_url(), "https://audio2.brain.fm/x.mp3?<signed>");
    }

    #[test]
    fn error_envelope_uses_messages_not_result() {
        let j = r#"{"status":422,"messages":["Invalid credentials"]}"#;
        let e: ErrorEnvelope = serde_json::from_str(j).unwrap();
        assert_eq!(e.status, 422);
        assert_eq!(e.lines(), vec!["Invalid credentials"]);
    }

    #[test]
    fn error_envelope_does_not_parse_as_success_envelope() {
        let j = r#"{"status":422,"messages":[]}"#;
        assert!(
            serde_json::from_str::<Envelope<Vec<MentalState>>>(j).is_err(),
            "the two envelopes are genuinely different shapes"
        );
    }
}
