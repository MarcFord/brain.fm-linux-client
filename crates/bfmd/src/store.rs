//! Where the session token lives.
//!
//! Only the token is ever persisted. The password is not stored under any
//! auth method — lazy renewal means we never need it again after the initial
//! exchange (see `docs/auth.md`).
//!
//! Preference order is the system keyring via Secret Service, falling back to
//! a `0600` file in a `0700` directory for headless hosts with no keyring
//! daemon. The fallback is a real downgrade, so it says so when it happens.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const SERVICE: &str = "dev.marcford.bfm";
const ACCOUNT: &str = "session-token";

/// Somewhere a session token can be kept.
pub trait TokenStore: Send + Sync {
    fn load(&self) -> Option<String>;
    fn save(&self, token: &str) -> std::io::Result<()>;
    fn clear(&self) -> std::io::Result<()>;
    /// For logs and `bfm whoami`.
    fn describe(&self) -> &'static str;
}

/// System keyring via Secret Service.
pub struct KeyringStore;

impl KeyringStore {
    fn entry() -> Option<keyring::v1::Entry> {
        keyring::v1::Entry::new(SERVICE, ACCOUNT).ok()
    }
}

impl TokenStore for KeyringStore {
    fn load(&self) -> Option<String> {
        Self::entry()?.get_password().ok().filter(|s| !s.is_empty())
    }

    fn save(&self, token: &str) -> std::io::Result<()> {
        Self::entry()
            .ok_or_else(|| std::io::Error::other("no keyring entry"))?
            .set_password(token)
            .map_err(std::io::Error::other)
    }

    fn clear(&self) -> std::io::Result<()> {
        match Self::entry() {
            // Already absent is success: clear() is idempotent.
            Some(e) => match e.delete_credential() {
                Ok(()) => Ok(()),
                Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(std::io::Error::other(e)),
            },
            None => Ok(()),
        }
    }

    fn describe(&self) -> &'static str {
        "system keyring"
    }
}

/// `0600` file in a `0700` directory, for hosts with no keyring daemon.
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `$XDG_STATE_HOME/bfm/token` (or `~/.local/state/bfm/token`).
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/state")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("bfm").join("token")
    }
}

impl TokenStore for FileStore {
    fn load(&self) -> Option<String> {
        let s = fs::read_to_string(&self.path).ok()?;
        let s = s.trim().to_owned();
        (!s.is_empty()).then_some(s)
    }

    fn save(&self, token: &str) -> std::io::Result<()> {
        #[cfg(unix)]
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
            #[cfg(unix)]
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }

        // Create with 0600 from the outset rather than widening then
        // narrowing, which would leave a window where the token is readable.
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        opts.mode(0o600);

        let mut f = opts.open(&self.path)?;
        f.write_all(token.as_bytes())?;
        f.flush()
    }

    fn clear(&self) -> std::io::Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn describe(&self) -> &'static str {
        "file (no keyring available)"
    }
}

/// In-memory store. Used by tests so they never touch the real keyring, and
/// by `--no-persist` runs.
#[derive(Default)]
pub struct MemoryStore {
    token: std::sync::Mutex<Option<String>>,
}

impl TokenStore for MemoryStore {
    fn load(&self) -> Option<String> {
        self.token.lock().ok()?.clone()
    }

    fn save(&self, token: &str) -> std::io::Result<()> {
        *self
            .token
            .lock()
            .map_err(|_| std::io::Error::other("memory store lock poisoned"))? =
            Some(token.to_owned());
        Ok(())
    }

    fn clear(&self) -> std::io::Result<()> {
        *self
            .token
            .lock()
            .map_err(|_| std::io::Error::other("memory store lock poisoned"))? = None;
        Ok(())
    }

    fn describe(&self) -> &'static str {
        "memory (not persisted)"
    }
}

/// Environment override for the token store: `keyring`, `file`, or `memory`.
///
/// Exists because probing for a keyring is not free — reading Secret Service
/// can *activate* `org.freedesktop.secrets`, spawning a keyring daemon. On a
/// desktop that is instant; in a container, over SSH, or on a private test
/// bus it can stall startup. Operators need a way to say "don't".
pub const STORE_ENV: &str = "BFM_TOKEN_STORE";

/// Pick a token store, preferring the keyring.
///
/// The keyring is probed lazily rather than at startup: a daemon that cannot
/// reach a keyring must still serve the bus, run timers and accept a login.
pub fn open() -> Box<dyn TokenStore> {
    match std::env::var(STORE_ENV).ok().as_deref() {
        Some("memory") => {
            tracing::info!("{STORE_ENV}=memory: the session will not be persisted");
            Box::new(MemoryStore::default())
        }
        Some("file") => Box::new(FileStore::new(FileStore::default_path())),
        Some("keyring") => Box::new(KeyringStore),
        Some(other) => {
            tracing::warn!("ignoring {STORE_ENV}={other:?}; expected keyring, file or memory");
            Box::new(LayeredStore::default())
        }
        None => Box::new(LayeredStore::default()),
    }
}

/// Keyring first, falling back to a file the moment the keyring misbehaves.
///
/// The fallback decision is made on first use and then remembered, so a
/// missing Secret Service costs one failed call rather than one per
/// operation.
pub struct LayeredStore {
    keyring: KeyringStore,
    file: FileStore,
    keyring_failed: std::sync::atomic::AtomicBool,
}

impl Default for LayeredStore {
    fn default() -> Self {
        Self {
            keyring: KeyringStore,
            file: FileStore::new(FileStore::default_path()),
            keyring_failed: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl LayeredStore {
    fn degraded(&self) -> bool {
        self.keyring_failed
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn degrade(&self) {
        if !self.degraded() {
            tracing::warn!(
                "no usable system keyring; falling back to a 0600 file at {}.                  Set {STORE_ENV}=file to skip this probe.",
                FileStore::default_path().display()
            );
            self.keyring_failed
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

impl TokenStore for LayeredStore {
    fn load(&self) -> Option<String> {
        // An empty keyring and a broken one look the same here, so fall
        // through to the file rather than guessing.
        if !self.degraded()
            && let Some(t) = self.keyring.load()
        {
            return Some(t);
        }
        self.file.load()
    }

    fn save(&self, token: &str) -> std::io::Result<()> {
        if !self.degraded() {
            match self.keyring.save(token) {
                Ok(()) => return Ok(()),
                Err(e) => tracing::debug!(error = %e, "keyring save failed"),
            }
            self.degrade();
        }
        self.file.save(token)
    }

    fn clear(&self) -> std::io::Result<()> {
        // Clear both: a token left in either place is a token still usable.
        let a = self.keyring.clear();
        let b = self.file.clear();
        a.and(b)
    }

    fn describe(&self) -> &'static str {
        if self.degraded() {
            "file (keyring unavailable)"
        } else {
            "system keyring"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("bfm-store-test-{}-{}", std::process::id(), name));
        p.push("token");
        p
    }

    #[test]
    fn file_store_round_trips() {
        let s = FileStore::new(temp("roundtrip"));
        assert!(s.load().is_none(), "nothing stored yet");

        s.save("a.b.c").unwrap();
        assert_eq!(s.load().as_deref(), Some("a.b.c"));

        s.clear().unwrap();
        assert!(s.load().is_none());
    }

    #[test]
    fn clearing_a_missing_token_is_not_an_error() {
        let s = FileStore::new(temp("idempotent"));
        s.clear().unwrap();
        s.clear().unwrap();
    }

    #[test]
    fn an_empty_file_reads_as_no_token() {
        let p = temp("empty");
        let s = FileStore::new(&p);
        s.save("   ").unwrap();
        assert!(s.load().is_none(), "whitespace is not a token");
        s.clear().unwrap();
    }

    #[test]
    fn a_saved_token_is_not_world_readable() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = temp("perms");
            let s = FileStore::new(&p);
            s.save("secret.jwt.here").unwrap();

            let mode = fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "token file must be owner-only, got {mode:o}");

            let dir = fs::metadata(p.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(
                dir, 0o700,
                "token directory must be owner-only, got {dir:o}"
            );
            s.clear().unwrap();
        }
    }

    #[test]
    fn saving_twice_replaces_rather_than_appends() {
        let p = temp("replace");
        let s = FileStore::new(&p);
        s.save("first.token.value").unwrap();
        s.save("second.tok").unwrap();
        assert_eq!(s.load().as_deref(), Some("second.tok"));
        s.clear().unwrap();
    }
}
