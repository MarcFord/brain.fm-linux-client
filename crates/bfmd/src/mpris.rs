//! MPRIS 2, so bfm behaves like any other player on the desktop.
//!
//! This buys media keys, `playerctl -p bfm`, waybar's built-in MPRIS module
//! and lock-screen widgets, all for state the daemon already holds. MPRIS is
//! deliberately generic, so it carries play/pause/next and metadata and
//! nothing Brain.fm-specific — the domain lives on `dev.marcford.BrainFM1`.
//!
//! State is not duplicated: these handlers read and write the single
//! `BrainFm` instance through the object server.

use std::collections::HashMap;

use zbus::{
    ObjectPath, Variant, interface,
    object_server::{ObjectServer, SignalEmitter},
};

use crate::iface::BrainFm;

pub const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";

/// `org.mpris.MediaPlayer2` — the root interface.
pub struct Root;

#[interface(interface = "org.mpris.MediaPlayer2")]
impl Root {
    /// There is no window to raise; the daemon has no UI of its own.
    async fn raise(&self) {}

    /// Quitting is systemd's job, not a media key's.
    async fn quit(&self) {}

    #[zbus(property)]
    async fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn identity(&self) -> String {
        "Brain.fm".to_owned()
    }

    #[zbus(property)]
    async fn desktop_entry(&self) -> String {
        "bfm".to_owned()
    }

    /// Nothing can be handed to us to play: tracks come from the service.
    #[zbus(property)]
    async fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    async fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

/// `org.mpris.MediaPlayer2.Player`.
pub struct Player;

/// Borrow the one `BrainFm` instance from the object server.
async fn brain(
    server: &ObjectServer,
) -> zbus::fdo::Result<zbus::object_server::InterfaceRef<BrainFm>> {
    server
        .interface::<_, BrainFm>(bfm_proto::OBJECT_PATH)
        .await
        .map_err(|e| zbus::fdo::Error::Failed(format!("bfm interface unavailable: {e}")))
}

#[interface(interface = "org.mpris.MediaPlayer2.Player")]
impl Player {
    async fn next(&self, #[zbus(object_server)] server: &ObjectServer) -> zbus::fdo::Result<()> {
        let r = brain(server).await?;
        let emitter = r.signal_emitter().clone();
        r.get_mut().await.skip_track(emitter).await
    }

    /// Brain.fm has no notion of a previous track — a session only moves
    /// forward — so this is a no-op and `CanGoPrevious` is false.
    async fn previous(&self) {}

    async fn play(&self, #[zbus(object_server)] server: &ObjectServer) -> zbus::fdo::Result<()> {
        let r = brain(server).await?;
        let emitter = r.signal_emitter().clone();
        r.get_mut().await.resume(emitter).await
    }

    async fn pause(&self, #[zbus(object_server)] server: &ObjectServer) -> zbus::fdo::Result<()> {
        let r = brain(server).await?;
        let emitter = r.signal_emitter().clone();
        r.get_mut().await.pause(emitter).await
    }

    /// What a media key actually sends.
    async fn play_pause(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<()> {
        let r = brain(server).await?;
        let emitter = r.signal_emitter().clone();
        let playing = r.get().await.playback == "playing";
        let mut iface = r.get_mut().await;
        if playing {
            iface.pause(emitter).await
        } else {
            iface.resume(emitter).await
        }
    }

    async fn stop(&self, #[zbus(object_server)] server: &ObjectServer) -> zbus::fdo::Result<()> {
        let r = brain(server).await?;
        let emitter = r.signal_emitter().clone();
        r.get_mut().await.stop_session(emitter).await
    }

    /// Seeking within functional music is not meaningful, and the decoder
    /// does not support it. `CanSeek` is false.
    async fn seek(&self, _offset: i64) {}

    async fn set_position(&self, _track: ObjectPath<'_>, _position: i64) {}

    async fn open_uri(&self, _uri: &str) {}

    #[zbus(property)]
    async fn playback_status(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<String> {
        let r = brain(server).await?;
        let s = r.get().await.playback;
        Ok(match s {
            "playing" => "Playing",
            "paused" => "Paused",
            // MPRIS has no "buffering"; a loading track reads as playing.
            "buffering" => "Playing",
            _ => "Stopped",
        }
        .to_owned())
    }

    #[zbus(property)]
    async fn metadata(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<HashMap<String, zbus::OwnedVariant>> {
        let r = brain(server).await?;
        let iface = r.get().await;
        let mut m: HashMap<String, zbus::OwnedVariant> = HashMap::new();

        let Some(t) = iface.track.as_ref() else {
            return Ok(m);
        };
        let mut put = |k: &str, v: Variant<'_>| {
            if let Ok(o) = zbus::OwnedVariant::try_from(v) {
                m.insert(k.to_owned(), o);
            }
        };

        // mpris:trackid must be a valid object path, so the id is reduced to
        // path-safe characters rather than interpolated raw.
        let safe: String =
            t.id.chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect();
        let path = format!("/dev/marcford/bfm/track/{safe}");
        if let Ok(p) = ObjectPath::try_from(path) {
            put("mpris:trackid", Variant::from(p));
        }
        // MPRIS lengths are microseconds.
        put("mpris:length", Variant::from(t.length as i64 * 1_000_000));
        put("xesam:title", Variant::from(t.name.clone()));
        put("xesam:artist", Variant::from(vec!["Brain.fm".to_owned()]));
        // The daemon has already cached this, so every MPRIS consumer gets
        // artwork without fetching anything itself.
        if !t.art_path.is_empty() {
            put(
                "mpris:artUrl",
                Variant::from(format!("file://{}", t.art_path)),
            );
        }
        Ok(m)
    }

    #[zbus(property)]
    async fn position(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<i64> {
        let r = brain(server).await?;
        let p = r.get().await.position;
        Ok(p as i64 * 1_000_000)
    }

    #[zbus(property)]
    async fn volume(&self, #[zbus(object_server)] server: &ObjectServer) -> zbus::fdo::Result<f64> {
        Ok(brain(server).await?.get().await.volume)
    }

    #[zbus(property)]
    async fn set_volume(
        &self,
        volume: f64,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<()> {
        // A property setter emits its own change notification.
        brain(server)
            .await?
            .get_mut()
            .await
            .set_volume(volume)
            .await;
        Ok(())
    }

    #[zbus(property)]
    async fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    async fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    async fn maximum_rate(&self) -> f64 {
        1.0
    }

    // Capabilities reflect what actually works, rather than claiming the
    // full spec and failing when a client takes us at our word.
    #[zbus(property)]
    async fn can_go_next(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn can_go_previous(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn can_seek(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn can_control(&self) -> bool {
        true
    }

    #[zbus(signal)]
    pub async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;
}
