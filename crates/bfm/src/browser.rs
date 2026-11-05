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

use std::path::{Path, PathBuf};

use base64::Engine as _;

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

/// Is this payload a Brain.fm session token rather than some other JWT?
///
/// Brain.fm's own token carries exactly `_id`, `iat` and `exp`. An SSO
/// provider's id token — which sits right next to it in the same storage —
/// carries `iss`, `aud` and friends. That difference is what lets us pick
/// the right one without guessing.
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

/// Pull every JWT-shaped run of bytes out of a file.
///
/// LevelDB interleaves keys, values and framing, so the files are scanned as
/// bytes rather than parsed. A real LevelDB reader would be tidier but would
/// also fight the lock Chrome holds while running; reading is enough.
fn scan_file(path: &Path, out: &mut Vec<String>) {
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
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

/// Find the freshest Brain.fm session token in any local browser profile.
pub fn find_session() -> Option<Found> {
    let mut best: Option<Found> = None;

    for (profile, dir) in candidate_profiles() {
        let mut jwts = Vec::new();
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = entry.path();
            // .log holds recent writes; .ldb holds compacted ones.
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "log" | "ldb") {
                scan_file(&p, &mut jwts);
            }
        }

        for jwt in jwts {
            let Some(payload) = decode_payload(&jwt) else {
                continue;
            };
            let Some(issued_at) = brainfm_claims(&payload) else {
                continue;
            };
            // Several logins can be left behind; the newest is the live one.
            if best.as_ref().is_none_or(|b| issued_at > b.issued_at) {
                best = Some(Found {
                    token: jwt,
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
    fn scans_a_jwt_out_of_surrounding_binary_noise() {
        let token = jwt(r#"{"_id":"u1","iat":5.0,"exp":9.0}"#);
        let mut blob = vec![0u8, 1, 2, 255, 7];
        blob.extend_from_slice(b"_https://my.brain.fm\x00\x01persist:auth");
        blob.extend_from_slice(token.as_bytes());
        blob.extend_from_slice(&[0u8, 3, 9]);

        let dir = std::env::temp_dir().join(format!("bfm-scan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("000003.log");
        std::fs::write(&f, &blob).unwrap();

        let mut out = Vec::new();
        scan_file(&f, &mut out);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(out.len(), 1, "should find exactly the one token");
        assert_eq!(out[0], token);
    }

    #[test]
    fn ignores_short_eyj_lookalikes() {
        let dir = std::env::temp_dir().join(format!("bfm-scan2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("000004.log");
        std::fs::write(&f, b"eyJa.b.c and eyJshort").unwrap();

        let mut out = Vec::new();
        scan_file(&f, &mut out);
        std::fs::remove_dir_all(&dir).ok();
        assert!(out.is_empty(), "too short to be a token");
    }
}
