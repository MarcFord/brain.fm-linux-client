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
    sync::{Arc, Mutex},
};

use keyring_core::api::CredentialStoreApi as _;

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

/// Why a keyring operation did not do what was asked.
///
/// The distinction matters most for `clear`. A keyring that answered "no
/// such entry" holds no token. One that could not be reached may well hold
/// one, and reporting that as a successful deletion is how `bfm logout`
/// prints "signed out" while a usable session stays behind.
#[derive(Debug)]
enum KeyringError {
    /// Secret Service could not be reached, or the connection to it broke.
    Unreachable(keyring_core::Error),
    /// Reached, but it would not do it: a locked collection, a dismissed
    /// unlock prompt, more than one matching item.
    Refused(keyring_core::Error),
}

impl std::fmt::Display for KeyringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(e) => write!(f, "the system keyring could not be reached: {e}"),
            Self::Refused(e) => write!(f, "the system keyring refused: {e}"),
        }
    }
}

impl std::error::Error for KeyringError {}

/// What the layered store needs from a keyring.
///
/// A trait so the layering can be tested against a fake whose reachability
/// the test controls; the real implementation talks to Secret Service.
trait Keyring: Send + Sync {
    fn load(&self) -> Result<Option<String>, KeyringError>;
    fn save(&self, token: &str) -> Result<(), KeyringError>;
    /// `Ok` means the keyring was reached and now holds no token.
    fn clear(&self) -> Result<(), KeyringError>;
}

/// System keyring via Secret Service.
///
/// Connects on first use and keeps the connection — but only a successful
/// one. This used to go through `keyring::v1::Entry`, which remembers the
/// *first* connection attempt for the life of the process, failure included.
/// A daemon that started before the keyring was ready could then never reach
/// it again, and every operation reported "no entry". For `load` and `save`
/// that merely meant falling back to the file. For `clear` it meant a logout
/// that found nothing to delete, called that success, and left the real
/// token in the keyring to be restored by the next daemon start. Here a
/// failed connection is forgotten and the next call tries again.
pub struct KeyringStore {
    store: Mutex<Option<Arc<zbus_secret_service_keyring_store::Store>>>,
}

impl Default for KeyringStore {
    fn default() -> Self {
        Self {
            store: Mutex::new(None),
        }
    }
}

impl KeyringStore {
    fn entry(&self) -> Result<keyring_core::Entry, KeyringError> {
        let mut cached = self.store.lock().unwrap_or_else(|p| p.into_inner());
        if cached.is_none() {
            let fresh = zbus_secret_service_keyring_store::Store::new()
                .map_err(KeyringError::Unreachable)?;
            *cached = Some(fresh);
        }
        cached
            .as_ref()
            .expect("set just above")
            .build(SERVICE, ACCOUNT, None)
            .map_err(KeyringError::Refused)
    }

    /// Classify a failure from an operation on an entry. A platform failure
    /// is the connection itself; drop it so the next call reconnects.
    fn failed(&self, e: keyring_core::Error) -> KeyringError {
        match e {
            keyring_core::Error::PlatformFailure(_) | keyring_core::Error::NoDefaultStore => {
                *self.store.lock().unwrap_or_else(|p| p.into_inner()) = None;
                KeyringError::Unreachable(e)
            }
            other => KeyringError::Refused(other),
        }
    }
}

impl Keyring for KeyringStore {
    fn load(&self) -> Result<Option<String>, KeyringError> {
        match self.entry()?.get_password() {
            Ok(s) if s.is_empty() => Ok(None),
            Ok(s) => Ok(Some(s)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(self.failed(e)),
        }
    }

    fn save(&self, token: &str) -> Result<(), KeyringError> {
        self.entry()?
            .set_password(token)
            .map_err(|e| self.failed(e))
    }

    fn clear(&self) -> Result<(), KeyringError> {
        match self.entry()?.delete_credential() {
            // Already absent is success: clear() is idempotent. The keyring
            // answered, so this is knowledge, not a guess.
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(self.failed(e)),
        }
    }
}

/// On its own (`BFM_TOKEN_STORE=keyring`) the keyring is the only place a
/// token may be, so a keyring that cannot be reached is an error, not an
/// absence.
impl TokenStore for KeyringStore {
    fn load(&self) -> Option<String> {
        match Keyring::load(self) {
            Ok(token) => token,
            Err(e) => {
                tracing::warn!(error = %e, "could not read the session from the system keyring");
                None
            }
        }
    }

    fn save(&self, token: &str) -> std::io::Result<()> {
        Keyring::save(self, token).map_err(std::io::Error::other)
    }

    fn clear(&self) -> std::io::Result<()> {
        Keyring::clear(self).map_err(std::io::Error::other)
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
        Some("keyring") => Box::new(KeyringStore::default()),
        Some(other) => {
            tracing::warn!("ignoring {STORE_ENV}={other:?}; expected keyring, file or memory");
            Box::new(LayeredStore::default())
        }
        None => Box::new(LayeredStore::default()),
    }
}

/// Keyring first, falling back to a file the moment the keyring misbehaves.
///
/// The fallback decision is made on the first failed save and then
/// remembered, so a missing Secret Service costs one failed call rather than
/// one per token rotation. `clear` is the exception: it always asks the
/// keyring, because the token a *previous* run put there outlives anything
/// this process has learnt about whether the keyring works.
///
/// Whether a session is in the keyring is recorded in a marker file beside
/// the fallback token file — written when a token is saved to or read from
/// the keyring, removed when the keyring confirms it holds none. It carries
/// no secret; it is what lets `clear` tell "the keyring is unreachable and
/// there is a session in it" (an error the user must see) from "the keyring
/// is unreachable and never held one" (a headless host, where failing every
/// logout would be noise).
pub struct LayeredStore {
    keyring: Box<dyn Keyring>,
    file: FileStore,
    /// Exists while a session is, or may be, in the keyring.
    marker: PathBuf,
    keyring_failed: std::sync::atomic::AtomicBool,
}

impl Default for LayeredStore {
    fn default() -> Self {
        Self::new(Box::new(KeyringStore::default()), FileStore::default_path())
    }
}

impl LayeredStore {
    fn new(keyring: Box<dyn Keyring>, token_file: PathBuf) -> Self {
        let marker = token_file.with_extension("in-keyring");
        Self {
            keyring,
            file: FileStore::new(token_file),
            marker,
            keyring_failed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn degraded(&self) -> bool {
        self.keyring_failed
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn degrade(&self) {
        if !self.degraded() {
            tracing::warn!(
                "no usable system keyring; falling back to a 0600 file at {}. \
                 Set {STORE_ENV}=file to skip this probe.",
                FileStore::default_path().display()
            );
            self.keyring_failed
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn session_in_keyring(&self) -> bool {
        self.marker.exists()
    }

    fn note_session_in_keyring(&self) {
        let written = self
            .marker
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(&self.marker, b""));
        if let Err(e) = written {
            tracing::debug!(error = %e, "could not record that the session is in the keyring");
        }
    }

    fn forget_session_in_keyring(&self) {
        match fs::remove_file(&self.marker) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::debug!(error = %e, "could not remove the keyring marker"),
        }
    }
}

impl TokenStore for LayeredStore {
    fn load(&self) -> Option<String> {
        if !self.degraded() {
            match self.keyring.load() {
                Ok(Some(t)) => {
                    self.note_session_in_keyring();
                    return Some(t);
                }
                // The keyring answered: nothing there. A marker from an
                // earlier run is stale.
                Ok(None) => self.forget_session_in_keyring(),
                Err(e) if self.session_in_keyring() => tracing::warn!(
                    error = %e,
                    "a session is stored in the system keyring but could not be read; \
                     it will be picked up once the keyring is reachable"
                ),
                Err(e) => tracing::debug!(error = %e, "keyring read failed"),
            }
        }
        self.file.load()
    }

    fn save(&self, token: &str) -> std::io::Result<()> {
        if !self.degraded() {
            match self.keyring.save(token) {
                Ok(()) => {
                    self.note_session_in_keyring();
                    // A token the file still holds from an earlier, degraded
                    // run is an older session that would stay usable. The
                    // live one is in the keyring now.
                    if let Err(e) = self.file.clear() {
                        tracing::debug!(error = %e, "could not remove the fallback token file");
                    }
                    return Ok(());
                }
                Err(e) => tracing::debug!(error = %e, "keyring save failed"),
            }
            self.degrade();
        }
        self.file.save(token)
    }

    fn clear(&self) -> std::io::Result<()> {
        // Clear both: a token left in either place is a token still usable.
        let file = self.file.clear();
        let keyring = match self.keyring.clear() {
            Ok(()) => {
                self.forget_session_in_keyring();
                Ok(())
            }
            Err(e) if self.session_in_keyring() => Err(std::io::Error::other(format!(
                "a session is stored in the system keyring and could not be removed \
                 ({e}); retry once the keyring is available"
            ))),
            Err(e) => {
                tracing::debug!(error = %e, "keyring unavailable, and no session was recorded there");
                Ok(())
            }
        };
        file.and(keyring)
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

    // ---- the layered store, against a keyring whose reachability the test controls ----

    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Default)]
    struct FakeKeyring {
        token: Mutex<Option<String>>,
        unreachable: AtomicBool,
    }

    impl FakeKeyring {
        fn set_reachable(&self, yes: bool) {
            self.unreachable.store(!yes, Ordering::Relaxed);
        }

        fn holds(&self) -> Option<String> {
            self.token.lock().unwrap().clone()
        }

        fn check(&self) -> Result<(), KeyringError> {
            if self.unreachable.load(Ordering::Relaxed) {
                Err(KeyringError::Unreachable(
                    keyring_core::Error::NoDefaultStore,
                ))
            } else {
                Ok(())
            }
        }
    }

    impl Keyring for Arc<FakeKeyring> {
        fn load(&self) -> Result<Option<String>, KeyringError> {
            self.check()?;
            Ok(self.holds())
        }

        fn save(&self, token: &str) -> Result<(), KeyringError> {
            self.check()?;
            *self.token.lock().unwrap() = Some(token.to_owned());
            Ok(())
        }

        fn clear(&self) -> Result<(), KeyringError> {
            self.check()?;
            *self.token.lock().unwrap() = None;
            Ok(())
        }
    }

    fn layered(name: &str) -> (LayeredStore, Arc<FakeKeyring>, PathBuf) {
        let keyring = Arc::new(FakeKeyring::default());
        let path = temp(name);
        let store = LayeredStore::new(Box::new(keyring.clone()), path.clone());
        (store, keyring, path)
    }

    fn cleanup(path: &Path) {
        if let Some(dir) = path.parent() {
            fs::remove_dir_all(dir).ok();
        }
    }

    /// The reported bug. A session goes into the keyring; later the keyring
    /// cannot be reached; `logout` must not claim success, because the token
    /// is still there and the next daemon start would restore it.
    #[test]
    fn logout_fails_rather_than_lie_when_the_keyring_holding_the_session_is_unreachable() {
        let (store, keyring, path) = layered("unreachable-logout");
        store.save("live.session.token").unwrap();
        assert_eq!(keyring.holds().as_deref(), Some("live.session.token"));

        keyring.set_reachable(false);
        let err = store.clear().expect_err("clear must not succeed");
        assert!(
            err.to_string().contains("system keyring"),
            "the error should say where the session is: {err}"
        );
        assert_eq!(
            keyring.holds().as_deref(),
            Some("live.session.token"),
            "the token is in fact still there"
        );

        // Secret Service comes back: the same logout now really clears it.
        keyring.set_reachable(true);
        store.clear().unwrap();
        assert_eq!(keyring.holds(), None);
        assert!(store.load().is_none());
        cleanup(&path);
    }

    /// A headless host: no keyring, ever. Logout must still work, or every
    /// sign-out there fails for a token that does not exist.
    #[test]
    fn an_unreachable_keyring_that_never_held_a_session_does_not_block_logout() {
        let (store, keyring, path) = layered("headless");
        keyring.set_reachable(false);

        store.save("file.only.token").unwrap();
        assert_eq!(store.load().as_deref(), Some("file.only.token"));
        assert_eq!(keyring.holds(), None);

        store.clear().unwrap();
        assert!(store.load().is_none());
        cleanup(&path);
    }

    /// A session saved by an earlier daemon (or an earlier version, before
    /// the marker existed) is recognised as living in the keyring the moment
    /// it is read back, so a later unreachable logout still refuses.
    #[test]
    fn a_session_restored_from_the_keyring_is_remembered_as_being_there() {
        let (store, keyring, path) = layered("restored");
        *keyring.token.lock().unwrap() = Some("older.daemon.token".into());

        assert_eq!(store.load().as_deref(), Some("older.daemon.token"));

        keyring.set_reachable(false);
        assert!(store.clear().is_err());
        assert_eq!(keyring.holds().as_deref(), Some("older.daemon.token"));
        cleanup(&path);
    }

    /// Once the keyring confirms it holds nothing, an unreachable keyring is
    /// no longer a reason to fail logout.
    #[test]
    fn a_confirmed_empty_keyring_clears_the_memory_of_a_session() {
        let (store, keyring, path) = layered("confirmed-empty");
        store.save("t.o.k").unwrap();
        store.clear().unwrap();

        keyring.set_reachable(false);
        store.clear().unwrap();
        cleanup(&path);
    }

    /// The fallback file is for when the keyring is down. When it is back and
    /// takes the new token, the file's older token must not linger as a
    /// second usable session.
    #[test]
    fn a_keyring_save_removes_a_stale_fallback_token() {
        let (store, keyring, path) = layered("stale-file");
        keyring.set_reachable(false);
        store.save("old.file.token").unwrap();

        // A fresh store: the degraded flag is per process.
        let store = LayeredStore::new(Box::new(keyring.clone()), path.clone());
        keyring.set_reachable(true);
        store.save("new.keyring.token").unwrap();

        assert_eq!(keyring.holds().as_deref(), Some("new.keyring.token"));
        assert!(
            FileStore::new(&path).load().is_none(),
            "the older token must be gone from the file"
        );
        cleanup(&path);
    }
}
