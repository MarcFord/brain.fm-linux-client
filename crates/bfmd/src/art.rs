//! Cover artwork, cached on disk.
//!
//! Fetching lives in the daemon rather than in each front-end for the same
//! reason everything else does: one place talks to Brain.fm, and every
//! client — GTK, Qt, a plugin — gets the result for free. It also means
//! MPRIS can advertise `mpris:artUrl`, so lock screens and media widgets
//! show the art without knowing anything about Brain.fm.
//!
//! **Images only.** Audio is never written to disk beyond the playback
//! buffer; artwork is ordinary media-player behaviour and a different thing
//! entirely.

use std::path::{Path, PathBuf};

/// Plenty for cover art, small enough that a surprising response cannot hurt.
const MAX_BYTES: usize = 4 * 1024 * 1024;

/// `$XDG_CACHE_HOME/bfm/art`, else `~/.cache/bfm/art`.
pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("bfm")
        .join("art")
}

/// FNV-1a. Inlined because the only requirement is that it is stable across
/// runs, which `DefaultHasher` explicitly is not.
fn hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Where a given image URL is cached. Deterministic, so a restart reuses
/// what is already on disk.
pub fn path_for(url: &str) -> PathBuf {
    let ext = Path::new(url)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| matches!(*e, "png" | "jpg" | "jpeg" | "webp"))
        .unwrap_or("img");
    cache_dir().join(format!("{:016x}.{ext}", hash(url)))
}

/// Return the cached path, downloading first if needed.
///
/// Returns `None` rather than erroring: missing artwork is a cosmetic
/// problem and must never interrupt playback.
pub async fn ensure(api: &bfm_api::Client, url: &str) -> Option<PathBuf> {
    if url.is_empty() {
        return None;
    }
    let path = path_for(url);
    if path.is_file() {
        return Some(path);
    }

    let bytes = match api.fetch_image(url, MAX_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            tracing::debug!(error = %e, "could not fetch cover art");
            return None;
        }
    };

    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        tracing::debug!(error = %e, "could not create the art cache directory");
        return None;
    }
    // Write beside and rename, so a half-written file is never read as art.
    let tmp = path.with_extension("part");
    if let Err(e) = std::fs::write(&tmp, &bytes) {
        tracing::debug!(error = %e, "could not write cover art");
        return None;
    }
    match std::fs::rename(&tmp, &path) {
        Ok(()) => Some(path),
        Err(e) => {
            tracing::debug!(error = %e, "could not install cover art");
            let _ = std::fs::remove_file(&tmp);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_url_always_maps_to_the_same_file() {
        let u = "https://cdn.brain.fm/images/tracks/topography.webp";
        assert_eq!(path_for(u), path_for(u));
    }

    #[test]
    fn different_urls_do_not_collide() {
        assert_ne!(
            path_for("https://cdn.brain.fm/a.webp"),
            path_for("https://cdn.brain.fm/b.webp")
        );
    }

    #[test]
    fn known_image_extensions_are_preserved() {
        for (url, ext) in [
            ("https://cdn.brain.fm/a.webp", "webp"),
            ("https://cdn.brain.fm/a.png", "png"),
            ("https://cdn.brain.fm/a.jpg", "jpg"),
        ] {
            assert_eq!(path_for(url).extension().unwrap(), ext, "{url}");
        }
    }

    /// A URL ending in something odd must not become a file with a
    /// surprising extension.
    #[test]
    fn unknown_extensions_are_normalised() {
        assert_eq!(path_for("https://x/a.php?id=1").extension().unwrap(), "img");
        assert_eq!(path_for("https://x/noext").extension().unwrap(), "img");
    }

    #[tokio::test]
    async fn an_empty_url_is_simply_no_art() {
        let api = bfm_api::Client::new().unwrap();
        assert!(ensure(&api, "").await.is_none());
    }
}
