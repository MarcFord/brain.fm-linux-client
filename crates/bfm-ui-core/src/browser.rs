//! Lifting an existing Brain.fm session out of the local browser profile.
//!
//! Accounts created with Google, Apple or Facebook cannot be authenticated
//! from a terminal (see `docs/auth.md`), so the provider login has to happen
//! in a real browser. Making the user then open DevTools and copy a value by
//! hand is a miserable way to finish that, and easy to get wrong — pasting
//! the snippet instead of its result produces a baffling base64 error.
//!
//! Chromium-family browsers keep `localStorage` in LevelDB files that contain
//! the value in plain text, so once the user has signed in we can simply read
//! it. No DevTools, no clipboard.
//!
//! This reads the user's own browser profile on their own machine, and only
//! ever looks for this one application's session.
//!
//! # Only Brain.fm's own records are considered
//!
//! `localStorage` for every site the user visits lives in the same files, and
//! the first version of this scanned those files for anything token-shaped
//! and told Brain.fm's tokens apart by their claims alone. That is not a
//! proof of origin: another site can issue a JWT with the same `_id`, `iat`
//! and `exp` claims, and a scan that trusted shape would have picked it up
//! and presented it to Brain.fm — sending one site's live credential to
//! another. So the files are now read as the LevelDB records they are, and a
//! token is only taken from a record whose key names a Brain.fm origin. The
//! claim check remains, but only to skip the SSO provider's own id token,
//! which Brain.fm's web app stores alongside the session.

use std::path::{Path, PathBuf};

use base64::Engine as _;

use crate::leveldb;

/// A session token found in a browser profile.
pub struct Found {
    pub token: String,
    /// `iat`, used to prefer the freshest when several are present.
    pub issued_at: f64,
    pub profile: String,
}

/// Chromium-family profiles worth searching, newest-looking first.
fn candidate_profiles() -> Vec<(String, PathBuf)> {
    let home = match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h),
        None => return Vec::new(),
    };
    let roots = [
        ("Chrome", ".config/google-chrome"),
        ("Chromium", ".config/chromium"),
        ("Brave", ".config/BraveSoftware/Brave-Browser"),
        ("Edge", ".config/microsoft-edge"),
        ("Vivaldi", ".config/vivaldi"),
    ];

    let mut out = Vec::new();
    for (name, rel) in roots {
        let root = home.join(rel);
        if !root.is_dir() {
            continue;
        }
        // "Default" plus any "Profile N".
        for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let p = entry.path();
            let fname = entry.file_name().to_string_lossy().to_string();
            if !p.is_dir() || (fname != "Default" && !fname.starts_with("Profile ")) {
                continue;
            }
            let ls = p.join("Local Storage").join("leveldb");
            if ls.is_dir() {
                out.push((format!("{name} ({fname})"), ls));
            }
        }
    }
    out
}

/// The origin a `localStorage` record belongs to, if it is one.
///
/// Chromium keys them `_` + storage key + `\0` + script key. For first-party
/// storage the storage key is the origin itself; a third-party partition
/// appends `^` and the top-level site, which we do not accept — a Brain.fm
/// frame embedded in someone else's page is not where the user signed in.
fn storage_origin(key: &[u8]) -> Option<&str> {
    let rest = key.strip_prefix(b"_")?;
    let end = rest.iter().position(|&b| b == 0)?;
    let origin = std::str::from_utf8(&rest[..end]).ok()?;
    if origin.contains('^') {
        return None;
    }
    Some(origin)
}

/// Is this origin one of Brain.fm's own, over HTTPS?
///
/// The web app lives at `my.brain.fm`; any host under `brain.fm` is accepted
/// so a move to another subdomain does not silently turn the profile read
/// off. Nothing outside that domain, no other scheme, no port.
fn is_brainfm_origin(origin: &str) -> bool {
    let Some(host) = origin.strip_prefix("https://") else {
        return false;
    };
    host == "brain.fm" || host.ends_with(".brain.fm")
}

/// A record value as bytes we can scan.
///
/// Chromium prefixes each value with its encoding: `0` for UTF-16LE, `1` for
/// Latin-1. The session is JSON, so the latter in practice, but a UTF-16
/// value is decoded rather than skipped: the token inside it is ASCII either
/// way.
fn decode_value(value: &[u8]) -> Option<Vec<u8>> {
    match value.split_first()? {
        (1, rest) => Some(rest.to_vec()),
        (0, rest) => {
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            Some(String::from_utf16_lossy(&units).into_bytes())
        }
        _ => None,
    }
}

/// Is this payload a Brain.fm session token rather than the SSO provider's?
///
/// Brain.fm's own token carries exactly `_id`, `iat` and `exp`. A provider's
/// id token — which the web app stores right next to it — carries `iss`,
/// `aud` and friends. This only tells the two apart within Brain.fm's own
/// records; it is not what decides whether a record is Brain.fm's at all.
fn brainfm_claims(payload: &serde_json::Value) -> Option<f64> {
    let obj = payload.as_object()?;
    if obj.contains_key("iss") || obj.contains_key("aud") {
        return None;
    }
    obj.get("_id")?.as_str()?;
    obj.get("exp")?.as_f64()?;
    obj.get("iat")?.as_f64()
}

fn decode_payload(jwt: &str) -> Option<serde_json::Value> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Pull every JWT-shaped run out of a record value.
///
/// The session sits inside a JSON wrapper the web app's state library
/// writes, so rather than depend on that wrapper's shape, look for the
/// token itself.
fn scan_jwts(bytes: &[u8], out: &mut Vec<String>) {
    let is_tok = |b: u8| b.is_ascii_alphanumeric() || b == b'-' || b == b'_';

    let mut i = 0;
    while i + 3 < bytes.len() {
        // JWTs begin with the base64 of '{"' — "eyJ".
        if &bytes[i..i + 3] != b"eyJ" {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        let mut dots = 0;
        while j < bytes.len() {
            match bytes[j] {
                b'.' => {
                    dots += 1;
                    if dots > 2 {
                        break;
                    }
                    j += 1;
                }
                b if is_tok(b) => j += 1,
                _ => break,
            }
        }
        if dots == 2
            && j - start > 60
            && let Ok(s) = std::str::from_utf8(&bytes[start..j])
        {
            out.push(s.to_owned());
        }
        i = j.max(start + 1);
    }
}

/// Every Brain.fm session token stored by one profile, with its `iat`.
fn sessions_in(dir: &Path) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for record in leveldb::read_dir(dir) {
        if !storage_origin(&record.key).is_some_and(is_brainfm_origin) {
            continue;
        }
        let Some(value) = decode_value(&record.value) else {
            continue;
        };
        let mut jwts = Vec::new();
        scan_jwts(&value, &mut jwts);
        for jwt in jwts {
            if let Some(issued_at) = decode_payload(&jwt).as_ref().and_then(brainfm_claims) {
                out.push((jwt, issued_at));
            }
        }
    }
    out
}

/// Find the freshest Brain.fm session token in any local browser profile.
pub fn find_session() -> Option<Found> {
    let mut best: Option<Found> = None;

    for (profile, dir) in candidate_profiles() {
        for (token, issued_at) in sessions_in(&dir) {
            // Several logins can be left behind; the newest is the live one.
            if best.as_ref().is_none_or(|b| issued_at > b.issued_at) {
                best = Some(Found {
                    token,
                    issued_at,
                    profile: profile.clone(),
                });
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(payload: &str) -> String {
        let b = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        format!(
            "{}.{}.{}",
            b(r#"{"alg":"RS256"}"#),
            b(payload),
            "c2lnbmF0dXJlX3BhZGRpbmc"
        )
    }

    /// `persist:auth` as the web app writes it: the token is a JSON string
    /// nested inside a JSON object, Latin-1 encoded.
    fn persisted(token: &str) -> Vec<u8> {
        let mut v = vec![1u8];
        v.extend_from_slice(format!(r#"{{"token":"\"{token}\"","user":"{{}}"}}"#).as_bytes());
        v
    }

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bfm-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn recognises_a_brainfm_token() {
        let v = decode_payload(&jwt(r#"{"_id":"u1","iat":100.0,"exp":1000.5}"#)).unwrap();
        assert_eq!(brainfm_claims(&v), Some(100.0));
    }

    /// The SSO provider's id token sits in the same storage. Picking it would
    /// authenticate nothing.
    #[test]
    fn rejects_a_provider_id_token() {
        let v = decode_payload(&jwt(
            r#"{"iss":"https://accounts.google.com","aud":"x","email":"a@b.c","iat":1,"exp":2}"#,
        ))
        .unwrap();
        assert_eq!(brainfm_claims(&v), None);
    }

    #[test]
    fn accepts_integer_and_float_timestamps() {
        let a =
            decode_payload(&jwt(r#"{"_id":"u","iat":1793917618,"exp":1793918518.095}"#)).unwrap();
        assert_eq!(brainfm_claims(&a), Some(1793917618.0));
    }

    #[test]
    fn reads_the_origin_out_of_a_record_key() {
        assert_eq!(
            storage_origin(b"_https://my.brain.fm\x00\x01persist:auth"),
            Some("https://my.brain.fm")
        );
        assert_eq!(storage_origin(b"META:https://my.brain.fm"), None);
        assert_eq!(storage_origin(b"VERSION"), None);
        assert_eq!(
            storage_origin(b"_https://my.brain.fm^0https://other.example\x00\x01k"),
            None,
            "a third-party partition is not where the user signed in"
        );
    }

    #[test]
    fn only_brainfm_origins_over_https_qualify() {
        assert!(is_brainfm_origin("https://my.brain.fm"));
        assert!(is_brainfm_origin("https://brain.fm"));
        assert!(is_brainfm_origin("https://www.brain.fm"));
        for bad in [
            "http://my.brain.fm",
            "https://my.brain.fm:8443",
            "https://brain.fm.example",
            "https://notbrain.fm",
            "https://my.brain.fm.evil.example",
            "https://other.example",
        ] {
            assert!(!is_brainfm_origin(bad), "{bad}");
        }
    }

    #[test]
    fn decodes_latin1_and_utf16_values() {
        assert_eq!(decode_value(b"\x01abc").unwrap(), b"abc");
        assert_eq!(decode_value(b"\x00a\x00b\x00c\x00").unwrap(), b"abc");
        assert!(decode_value(b"\x07abc").is_none(), "unknown encoding");
        assert!(decode_value(b"").is_none());
    }

    #[test]
    fn scans_a_jwt_out_of_surrounding_noise() {
        let token = jwt(r#"{"_id":"u1","iat":5.0,"exp":9.0}"#);
        let mut blob = vec![0u8, 1, 2, 255, 7];
        blob.extend_from_slice(b"\"token\":\"\\\"");
        blob.extend_from_slice(token.as_bytes());
        blob.extend_from_slice(b"\\\"\"}");

        let mut out = Vec::new();
        scan_jwts(&blob, &mut out);
        assert_eq!(out, vec![token]);
    }

    #[test]
    fn ignores_short_eyj_lookalikes() {
        let mut out = Vec::new();
        scan_jwts(b"eyJa.b.c and eyJshort", &mut out);
        assert!(out.is_empty(), "too short to be a token");
    }

    /// The whole point of reading records rather than bytes. Another site's
    /// token with exactly Brain.fm's claim shape, and fresher, must not win —
    /// or even be considered — because its record belongs to another origin.
    #[test]
    fn a_lookalike_token_from_another_origin_is_never_taken() {
        let ours = jwt(r#"{"_id":"5f1e","iat":100,"exp":200}"#);
        let theirs = jwt(r#"{"_id":"user-9","iat":999,"exp":2000}"#);
        let dir = scratch_dir("origin");
        std::fs::write(
            dir.join("000003.log"),
            leveldb::tests::log_file(
                &[
                    (
                        b"_https://other.example\x00\x01persist:auth",
                        Some(&persisted(&theirs)),
                    ),
                    (
                        b"_https://my.brain.fm\x00\x01persist:auth",
                        Some(&persisted(&ours)),
                    ),
                ],
                usize::MAX,
            ),
        )
        .unwrap();

        let found = sessions_in(&dir);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(found, vec![(ours, 100.0)]);
    }

    #[test]
    fn nothing_is_found_when_only_other_origins_hold_tokens() {
        let theirs = jwt(r#"{"_id":"user-9","iat":999,"exp":2000}"#);
        let dir = scratch_dir("foreign");
        std::fs::write(
            dir.join("000003.log"),
            leveldb::tests::log_file(
                &[(
                    b"_https://other.example\x00\x01session",
                    Some(&persisted(&theirs)),
                )],
                usize::MAX,
            ),
        )
        .unwrap();
        // The bare token also appears in a table, as a compacted write would.
        std::fs::write(
            dir.join("000005.ldb"),
            leveldb::tests::table_file(
                &[(
                    b"_https://another.example\x00\x01jwt",
                    1,
                    &persisted(&theirs),
                )],
                true,
            ),
        )
        .unwrap();

        let found = sessions_in(&dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(found.is_empty(), "{}", found.len());
    }

    /// After compaction the session lives only in a table, and Chromium
    /// compresses those. Reading it there is what makes the profile read
    /// work for anyone whose browser has been open a while.
    #[test]
    fn finds_the_session_in_a_compressed_table() {
        let ours = jwt(r#"{"_id":"5f1e","iat":100,"exp":200}"#);
        let sso = jwt(r#"{"iss":"https://accounts.google.com","aud":"x","iat":150,"exp":300}"#);
        let dir = scratch_dir("table");
        std::fs::write(
            dir.join("000005.ldb"),
            leveldb::tests::table_file(
                &[
                    (b"_https://my.brain.fm\x00\x01google", 1, &persisted(&sso)),
                    (
                        b"_https://my.brain.fm\x00\x01persist:auth",
                        1,
                        &persisted(&ours),
                    ),
                ],
                true,
            ),
        )
        .unwrap();

        let found = sessions_in(&dir);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(found, vec![(ours, 100.0)]);
    }
}
