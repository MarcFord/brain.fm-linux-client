//! Rust side of the Qt bridge.
//!
//! The same daemon, the same `bfm-ui-core`, the same words — only the
//! widgets differ. Every string QML renders is computed by
//! `bfm_ui_core::display`, which the GTK front-end also uses, so the two
//! cannot drift into describing the same state differently.
//!
//! QML binds to properties, so the bridge exposes finished text rather than
//! raw state: no logic belongs in a `.qml` file, where it would be
//! untestable and duplicated.

use bfm_ui_core::{Client, Command, SessionMode, State, Update, display};
use cxx_qt::{CxxQtType as _, Threading as _};
use cxx_qt_lib::{QList, QString, QStringList};

/// QStringList is only constructible from a QList, so build one.
fn string_list<'a>(items: impl IntoIterator<Item = &'a str>) -> QStringList {
    let mut list = QList::<QString>::default();
    for s in items {
        list.append(QString::from(s));
    }
    QStringList::from(&list)
}

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        // Which screen to show: connecting | signin | browse | player.
        #[qproperty(QString, page)]
        #[qproperty(bool, connected)]
        #[qproperty(bool, authenticated)]
        // Finished display strings, computed in Rust.
        #[qproperty(QString, track_name)]
        #[qproperty(QString, track_detail)]
        #[qproperty(QString, position_label)]
        #[qproperty(QString, timer_label)]
        #[qproperty(QString, mental_state)]
        #[qproperty(f64, track_fraction)]
        #[qproperty(bool, is_favorite)]
        #[qproperty(QString, art_path)]
        #[qproperty(f64, volume)]
        #[qproperty(QString, banner_text)]
        #[qproperty(bool, banner_is_error)]
        // Catalog for the wizard, refreshed by fetch_catalog.
        #[qproperty(QStringList, activity_names)]
        #[qproperty(QStringList, genre_names)]
        #[qproperty(QStringList, length_labels)]
        #[qproperty(bool, catalog_ready)]
        type Session = super::SessionRust;
    }

    unsafe extern "RustQt" {
        #[qinvokable]
        fn start(self: Pin<&mut Session>, state: &QString);

        /// Start with everything the wizard collected. `genres` is a
        /// newline-separated list, because QML string lists cross the
        /// bridge awkwardly and this is only ever built by our own QML.
        #[qinvokable]
        fn start_configured(
            self: Pin<&mut Session>,
            state: &QString,
            activity_index: i32,
            effort: &QString,
            genres: &QString,
            timer_spec: &QString,
        );

        #[qinvokable]
        fn play_pause(self: Pin<&mut Session>);
        #[qinvokable]
        fn skip(self: Pin<&mut Session>);
        #[qinvokable]
        fn stop(self: Pin<&mut Session>);
        #[qinvokable]
        fn toggle_favorite(self: Pin<&mut Session>);
        #[qinvokable]
        fn dislike(self: Pin<&mut Session>);
        #[qinvokable]
        fn sign_in(self: Pin<&mut Session>);
        #[qinvokable]
        fn apply_volume(self: Pin<&mut Session>, volume: f64);
        #[qinvokable]
        fn set_timer_spec(self: Pin<&mut Session>, spec: &QString);
        #[qinvokable]
        fn fetch_catalog(self: Pin<&mut Session>, state: &QString);
        #[qinvokable]
        fn dismiss_banner(self: Pin<&mut Session>);

        /// Text for the share action. Qt copies it to the clipboard in QML.
        #[qinvokable]
        fn share_text(self: Pin<&mut Session>) -> QString;

        /// Seconds for the suggested length at `index`, for the wizard.
        #[qinvokable]
        fn length_seconds(self: Pin<&mut Session>, index: i32) -> i32;

        /// The step after `current`, given the chosen length mode.
        /// Returns -1 when there is nothing left to ask.
        ///
        /// Exposed so QML follows the same tested sequencing the GTK wizard
        /// does, instead of hard-coding a second copy of it.
        #[qinvokable]
        fn step_after(self: Pin<&mut Session>, current: i32, mode_index: i32) -> i32;

        /// Begin listening to the daemon. Called once from QML.
        #[qinvokable]
        fn connect_daemon(self: Pin<&mut Session>);
    }

    impl cxx_qt::Threading for Session {}
}

pub struct SessionRust {
    page: QString,
    connected: bool,
    authenticated: bool,
    track_name: QString,
    track_detail: QString,
    position_label: QString,
    timer_label: QString,
    mental_state: QString,
    track_fraction: f64,
    is_favorite: bool,
    art_path: QString,
    volume: f64,
    banner_text: QString,
    banner_is_error: bool,
    activity_names: QStringList,
    genre_names: QStringList,
    length_labels: QStringList,
    catalog_ready: bool,

    client: Option<Box<Client>>,
    state: State,
    mode: SessionMode,
    catalog: Option<bfm_ui_core::Catalog>,
    error: Option<String>,
    notice: Option<String>,
}

impl Default for SessionRust {
    fn default() -> Self {
        Self {
            page: QString::from("connecting"),
            connected: false,
            authenticated: false,
            track_name: QString::from(""),
            track_detail: QString::from(""),
            position_label: QString::from("0:00 / 0:00"),
            timer_label: QString::from(""),
            mental_state: QString::from(""),
            track_fraction: 0.0,
            is_favorite: false,
            art_path: QString::from(""),
            volume: 1.0,
            banner_text: QString::from(""),
            banner_is_error: false,
            activity_names: QStringList::default(),
            genre_names: QStringList::default(),
            length_labels: QStringList::default(),
            catalog_ready: false,
            client: None,
            state: State::default(),
            mode: SessionMode::default(),
            catalog: None,
            error: None,
            notice: None,
        }
    }
}

impl qobject::Session {
    fn send(self: std::pin::Pin<&mut Self>, cmd: Command) {
        if let Some(c) = self.client.as_ref() {
            c.send(cmd);
        }
    }

    /// Push every derived string onto the Qt properties.
    ///
    /// One function so a new field cannot be wired into some code paths and
    /// forgotten in others.
    fn republish(mut self: std::pin::Pin<&mut Self>) {
        let s = self.state.clone();
        let mode = self.mode;

        let banner = display::banner(
            s.daemon_is_stale().then(|| s.stale_daemon_message()),
            self.error.as_deref(),
            self.notice.as_deref(),
        );
        let (is_error, text) = banner.unwrap_or((false, String::new()));

        let page = display::page(self.connected, s.authenticated, s.playback);
        let length = s.track.as_ref().map(|t| t.length).unwrap_or(0);

        self.as_mut().set_page(QString::from(page));
        self.as_mut().set_authenticated(s.authenticated);
        self.as_mut()
            .set_track_name(QString::from(&display::track_name(s.track.as_ref())));
        self.as_mut()
            .set_track_detail(QString::from(&display::track_detail(
                s.track.as_ref(),
                s.playback,
            )));
        self.as_mut()
            .set_position_label(QString::from(&display::position_label(s.position, length)));
        self.as_mut()
            .set_timer_label(QString::from(&display::timer_label(&s.timer, mode)));
        self.as_mut()
            .set_mental_state(QString::from(&display::title_case(&s.mental_state)));
        self.as_mut().set_track_fraction(s.track_fraction());
        self.as_mut()
            .set_is_favorite(s.track.as_ref().is_some_and(|t| t.is_favorite));
        // QML's Image wants a URL, and a missing file should show nothing.
        self.as_mut().set_art_path(QString::from(
            &s.art_file()
                .map(|p| format!("file://{p}"))
                .unwrap_or_default(),
        ));
        self.as_mut().set_volume(s.volume);
        self.as_mut().set_banner_text(QString::from(&text));
        self.as_mut().set_banner_is_error(is_error);
    }

    fn apply_update(mut self: std::pin::Pin<&mut Self>, update: Update) {
        match update {
            Update::State(s) => {
                self.as_mut().set_connected(true);
                self.as_mut().rust_mut().state = *s;
                if self.state.authenticated {
                    self.as_mut().rust_mut().error = None;
                }
            }
            Update::Disconnected(why) => {
                self.as_mut().set_connected(false);
                self.as_mut().rust_mut().error =
                    Some(format!("Cannot reach the Brain.fm daemon: {why}"));
            }
            Update::Failed { code, message } => {
                self.as_mut().rust_mut().notice = None;
                // Signing in is a screen, not an error.
                self.as_mut().rust_mut().error =
                    (!bfm_ui_core::needs_login(&code)).then_some(message);
            }
            Update::Catalog(catalog) => {
                let names = string_list(catalog.activities.iter().map(|(_, n)| n.as_str()));
                let genres = string_list(catalog.genres.iter().map(String::as_str));
                let lengths = string_list(catalog.lengths.iter().map(|(_, l)| l.as_str()));

                self.as_mut().set_activity_names(names);
                self.as_mut().set_genre_names(genres);
                self.as_mut().set_length_labels(lengths);
                self.as_mut().rust_mut().catalog = Some(*catalog);
                self.as_mut().set_catalog_ready(true);
            }
        }
        self.republish();
    }
}

impl qobject::Session {
    /// Start the bus client and pump its updates onto the Qt thread.
    ///
    /// `bfm-ui-core` runs the bus on its own thread with its own runtime;
    /// `CxxQtThread` is how its results reach a QObject safely. Nothing
    /// touches a Qt property off-thread.
    pub fn connect_daemon(mut self: std::pin::Pin<&mut Self>) {
        if self.client.is_some() {
            return;
        }
        let client = Client::spawn();
        let updates = client.updates();
        self.as_mut().rust_mut().client = Some(Box::new(client));

        let qt = self.qt_thread();
        std::thread::Builder::new()
            .name("bfm-qt-pump".into())
            .spawn(move || {
                while let Ok(update) = updates.recv_blocking() {
                    if qt
                        .queue(move |session| session.apply_update(update))
                        .is_err()
                    {
                        break; // the object is gone
                    }
                }
            })
            .expect("spawn qt pump");
    }

    pub fn start(mut self: std::pin::Pin<&mut Self>, state: &QString) {
        let state = state.to_string();
        self.as_mut().send(Command::StartSession {
            state,
            activity: String::new(),
            genres: Vec::new(),
            effort: "high".into(),
            timer: String::new(),
        });
    }

    pub fn start_configured(
        mut self: std::pin::Pin<&mut Self>,
        state: &QString,
        activity_index: i32,
        effort: &QString,
        genres: &QString,
        timer_spec: &QString,
    ) {
        let activity = self
            .catalog
            .as_ref()
            .and_then(|c| c.activity_id(activity_index.max(0) as u32))
            .unwrap_or_default()
            .to_owned();

        // Newline-separated, built by our own QML; empty means no filter,
        // which is not the same as every genre.
        let genres: Vec<String> = genres
            .to_string()
            .lines()
            .filter(|g| !g.trim().is_empty())
            .map(str::to_owned)
            .collect();

        let spec = timer_spec.to_string();
        if let Some(mode) = SessionMode::from_spec(&spec) {
            self.as_mut().rust_mut().mode = mode;
        }

        self.as_mut().send(Command::StartSession {
            state: state.to_string(),
            activity,
            genres,
            effort: effort.to_string(),
            timer: spec,
        });
    }

    pub fn play_pause(mut self: std::pin::Pin<&mut Self>) {
        let cmd = if self.state.playback.toggles_to_play() {
            Command::Resume
        } else {
            Command::Pause
        };
        self.as_mut().send(cmd);
    }

    pub fn skip(mut self: std::pin::Pin<&mut Self>) {
        self.as_mut().send(Command::Skip);
    }

    pub fn stop(mut self: std::pin::Pin<&mut Self>) {
        self.as_mut().send(Command::Stop);
    }

    pub fn toggle_favorite(mut self: std::pin::Pin<&mut Self>) {
        let on = !self.state.track.as_ref().is_some_and(|t| t.is_favorite);
        self.as_mut().send(Command::Favorite(on));
    }

    pub fn dislike(mut self: std::pin::Pin<&mut Self>) {
        self.as_mut().send(Command::Dislike);
    }

    pub fn apply_volume(mut self: std::pin::Pin<&mut Self>, volume: f64) {
        self.as_mut()
            .send(Command::SetVolume(volume.clamp(0.0, 1.0)));
    }

    pub fn set_timer_spec(mut self: std::pin::Pin<&mut Self>, spec: &QString) {
        let spec = spec.to_string();
        if let Some(mode) = SessionMode::from_spec(&spec) {
            self.as_mut().rust_mut().mode = mode;
        }
        self.as_mut().send(Command::SetTimer(spec));
    }

    pub fn fetch_catalog(mut self: std::pin::Pin<&mut Self>, state: &QString) {
        self.as_mut().set_catalog_ready(false);
        let state = state.to_string();
        self.as_mut().send(Command::FetchCatalog(state));
    }

    pub fn sign_in(mut self: std::pin::Pin<&mut Self>) {
        match bfm_ui_core::find_session() {
            Some(found) => {
                self.as_mut()
                    .send(Command::LoginFromBrowserToken(found.token));
            }
            None => {
                self.as_mut().rust_mut().error = Some(
                    "No Brain.fm session found in a local browser. Sign in at \
                     my.brain.fm, close the tab so the browser writes it to disk, \
                     then try again."
                        .into(),
                );
                self.republish();
            }
        }
    }

    pub fn share_text(mut self: std::pin::Pin<&mut Self>) -> QString {
        let Some(track) = self.state.track.clone() else {
            return QString::from("");
        };
        let text = bfm_ui_core::share_text(&track, &self.state.mental_state);
        self.as_mut().rust_mut().notice = Some("Track details copied".into());
        self.as_mut().republish();
        QString::from(&text)
    }

    pub fn length_seconds(self: std::pin::Pin<&mut Self>, index: i32) -> i32 {
        self.catalog
            .as_ref()
            .and_then(|c| c.lengths.get(index.max(0) as usize))
            .map(|(secs, _)| *secs as i32)
            .unwrap_or(0)
    }

    pub fn step_after(self: std::pin::Pin<&mut Self>, current: i32, mode_index: i32) -> i32 {
        use bfm_ui_core::steps::{self, Step};

        let step = match current {
            0 => Step::Activity,
            1 => Step::Effort,
            2 => Step::Genres,
            3 => Step::LengthKind,
            4 => Step::TimerLength,
            _ => Step::IntervalSetup,
        };
        let mode = match mode_index {
            1 => SessionMode::Timer { minutes: 50 },
            2 => SessionMode::Intervals {
                work: 25,
                rest: 5,
                cycles: 4,
            },
            _ => SessionMode::Infinite,
        };
        match steps::next(step, mode) {
            None => -1,
            Some(Step::Activity) => 0,
            Some(Step::Effort) => 1,
            Some(Step::Genres) => 2,
            Some(Step::LengthKind) => 3,
            Some(Step::TimerLength) => 4,
            Some(Step::IntervalSetup) => 5,
        }
    }

    pub fn dismiss_banner(mut self: std::pin::Pin<&mut Self>) {
        self.as_mut().rust_mut().error = None;
        self.as_mut().rust_mut().notice = None;
        self.republish();
    }
}
