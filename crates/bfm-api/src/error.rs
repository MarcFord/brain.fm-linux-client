use std::fmt;

pub type Result<T> = std::result::Result<T, ApiError>;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Credentials were rejected, or the token is past the (unmeasured) renewal
    /// grace window. The only correct response is to log in again.
    #[error("not authenticated; log in again")]
    Unauthenticated,

    /// The token was malformed. Note the API answers 400, not 401, for this.
    #[error("the API rejected the token as malformed")]
    MalformedToken,

    #[error("rate limited by the API{}", match .retry_after_secs {
        Some(s) => format!("; retry after {s}s"),
        None => String::new(),
    })]
    RateLimited { retry_after_secs: Option<u64> },

    #[error("api returned HTTP {status}")]
    Http { status: u16 },

    /// A deserialize failure almost always means Brain.fm changed the schema,
    /// not that the caller did something wrong. Say so.
    #[error(
        "the Brain.fm API returned a shape we don't recognise \
             (it likely changed); at {context}: {source}"
    )]
    SchemaChanged {
        context: &'static str,
        #[source]
        source: serde_json::Error,
    },

    #[error("could not decode the session token: {0}")]
    BadTokenEncoding(String),

    #[error(transparent)]
    Transport(#[from] reqwest::Error),
}

impl ApiError {
    /// True when the caller should prompt for a fresh login rather than retry.
    pub fn needs_login(&self) -> bool {
        matches!(self, Self::Unauthenticated | Self::MalformedToken)
    }
}

/// Wrapper that keeps signed URLs and bearer tokens out of logs.
///
/// A `tokenedUrl` is a bearer credential for the audio. Printing one into a bug
/// report hands it to whoever reads the report, so the only `Debug`/`Display`
/// this type offers is redacted.
#[derive(Clone)]
pub struct Redacted<T>(pub T);

impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl<T> fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Strip the signing parameters from an audio URL so it is safe to log.
pub fn scrub_audio_url(url: &str) -> String {
    match url.split_once('?') {
        Some((base, _)) => format!("{base}?<signed>"),
        None => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_signing_params() {
        let u = "https://audio2.brain.fm/abc.mp3?expiration=1801687490428&token=deadbeef";
        assert_eq!(
            scrub_audio_url(u),
            "https://audio2.brain.fm/abc.mp3?<signed>"
        );
    }

    #[test]
    fn unsigned_url_survives_scrubbing() {
        let u = "https://audio2.brain.fm/abc.mp3";
        assert_eq!(scrub_audio_url(u), u);
    }

    #[test]
    fn redacted_never_prints_its_contents() {
        let r = Redacted("hunter2");
        assert_eq!(format!("{r:?}"), "<redacted>");
        assert_eq!(format!("{r}"), "<redacted>");
    }
}
