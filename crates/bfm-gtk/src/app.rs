//! The window.

use crate::{view, wizard};
use bfm_ui_core::{
    Catalog, Client, Command, Filters, Playback, SessionMode, State, Update, needs_login,
    share_text,
};
use relm4::{
    ComponentParts, ComponentSender, RelmWidgetExt, SimpleComponent,
    adw::{self, prelude::*},
    gtk,
};

/// The four mental states, in the order the web app shows them.
///
/// Hard-coded rather than fetched: the slugs are stable, and a sign-in screen
/// that cannot even list the modes until you have signed in is a poor one.
const STATES: [(&str, &str, &str); 4] = [
    ("focus", "Focus", "Deep concentration"),
    ("relax", "Relax", "Wind down"),
    ("sleep", "Sleep", "Drift off"),
    ("meditate", "Meditate", "Settle the mind"),
];

pub struct Init {
    pub layer: bool,
}

pub struct App {
    client: Client,
    state: State,
    /// `None` until the daemon answers, so the UI can say "connecting"
    /// instead of pretending to be idle.
    connected: bool,
    error: Option<String>,
    /// How long the next session should run. Applied immediately when one is
    /// already playing, so the control means the same thing on both pages.
    mode: SessionMode,
    /// Kept so a transient dialog has something to be presented on.
    window: adw::ApplicationWindow,
    /// Choices for the next session. Per-state, so they reset when the
    /// state changes rather than carrying Sleep's genres into Focus.
    filters: Filters,
    /// What to open once the catalog arrives.
    pending: Option<Pending>,
    /// Last catalog fetched, so changing the length mid-session does not
    /// need another round trip.
    catalog: Option<Catalog>,
    /// A short confirmation, distinct from an error.
    notice: Option<String>,
    /// Set once the view has been built and the stack has its pages.
    stack_ready: bool,
}

/// Which wizard is waiting on a catalog fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Start(&'static str),
    Length,
}

#[derive(Debug)]
pub enum Msg {
    FromDaemon(Box<Update>),
    /// A state was tapped: load its catalog, then offer the editor.
    ConfigureStart(&'static str),
    /// Start with everything the editor collected.
    StartConfigured {
        state: &'static str,
        filters: Box<Filters>,
        mode: SessionMode,
    },
    PlayPause,
    Skip,
    Stop,
    SignIn,
    Volume(f64),
    OpenModeDialog,
    SetMode(SessionMode),
    ToggleFavorite,
    Dislike,
    Share,
    DismissBanner,
}

#[relm4::component(pub)]
impl SimpleComponent for App {
    type Init = Init;
    type Input = Msg;
    type Output = ();

    view! {
        adw::ApplicationWindow {
            set_title: Some("Brain.fm"),
            set_default_size: (420, 560),

            adw::ToolbarView {
                add_top_bar = &adw::HeaderBar {
                    #[wrap(Some)]
                    set_title_widget = &adw::WindowTitle {
                        set_title: "Brain.fm",
                        #[watch]
                        set_subtitle: &model.subtitle(),
                    },

                    pack_end = &gtk::Button {
                        set_icon_name: "alarm-symbolic",
                        set_tooltip_text: Some("Session length"),
                        #[watch]
                        set_visible: model.connected && model.state.authenticated,
                        connect_clicked => Msg::OpenModeDialog,
                    },
                },

                #[wrap(Some)]
                set_content = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,

                    // Errors appear in place rather than as a transient
                    // toast: "sign in" is a state, not a notification.
                    gtk::Revealer {
                        #[watch]
                        set_reveal_child: model.has_banner(),
                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 8,
                            set_margin_all: 8,
                            add_css_class: "card",

                            gtk::Image {
                                #[watch]
                                set_icon_name: Some(if model.banner_is_error() {
                                    "dialog-warning-symbolic"
                                } else {
                                    "object-select-symbolic"
                                }),
                            },
                            gtk::Label {
                                set_wrap: true,
                                set_xalign: 0.0,
                                set_hexpand: true,
                                #[watch]
                                set_label: &model.banner_text(),
                            },
                            gtk::Button {
                                set_icon_name: "window-close-symbolic",
                                add_css_class: "flat",
                                connect_clicked => Msg::DismissBanner,
                            },
                        },
                    },

                    gtk::Stack {
                        set_vexpand: true,
                        set_transition_type: gtk::StackTransitionType::Crossfade,
                        // Assigned after the children below: relm4 applies
                        // properties before adding named children, so
                        // selecting a page up here names one that does not
                        // exist yet.
                        #[watch]
                        set_visible_child_name?: model.page_if_ready(),

                        // ---- connecting ----
                        add_named[Some("connecting")] = &adw::StatusPage {
                            set_icon_name: Some("content-loading-symbolic"),
                            set_title: "Connecting",
                            set_description: Some("Starting the Brain.fm daemon…"),
                        },

                        // ---- signed out ----
                        add_named[Some("signin")] = &adw::StatusPage {
                            set_icon_name: Some("avatar-default-symbolic"),
                            set_title: "Sign in",
                            set_description: Some(
                                "Sign in to Brain.fm in your browser, then pick up the session here."
                            ),
                            #[wrap(Some)]
                            set_child = &gtk::Button {
                                set_label: "Use my browser session",
                                set_halign: gtk::Align::Center,
                                add_css_class: "pill",
                                add_css_class: "suggested-action",
                                connect_clicked => Msg::SignIn,
                            },
                        },

                        // ---- choose a mode ----
                        add_named[Some("browse")] = &gtk::ScrolledWindow {
                            set_hscrollbar_policy: gtk::PolicyType::Never,
                            adw::Clamp {
                                set_maximum_size: 420,
                                gtk::Box {
                                    set_orientation: gtk::Orientation::Vertical,
                                    set_spacing: 12,
                                    set_margin_all: 16,

                                    adw::PreferencesGroup {
                                        adw::ActionRow {
                                            set_title: "Session length",
                                            #[watch]
                                            set_subtitle: &model.mode.to_string(),
                                            set_activatable: true,
                                            add_suffix = &gtk::Image {
                                                set_icon_name: Some("go-next-symbolic"),
                                            },
                                            connect_activated => Msg::OpenModeDialog,
                                        },
                                    },

                                    adw::PreferencesGroup {
                                        set_title: "Start a session",

                                        #[name = "state_rows"]
                                        adw::PreferencesGroup {},
                                    },
                                },
                            },
                        },

                        // ---- playing ----
                        //
                        // The track's own artwork sits behind the controls,
                        // the way the web player does it. A scrim keeps text
                        // legible over whatever image arrives — they range
                        // from near-black to washed-out, so readability
                        // cannot be left to luck.
                        add_named[Some("player")] = &gtk::Overlay {
                            #[name = "art"]
                            gtk::Picture {
                                set_content_fit: gtk::ContentFit::Cover,
                                set_can_shrink: true,
                                #[watch]
                                set_filename: model.state.art_file(),
                                #[watch]
                                set_visible: model.state.art_file().is_some(),
                            },

                            add_overlay = &gtk::Box {
                                #[watch]
                                set_css_classes: model.scrim_classes(),

                                adw::Clamp {
                                set_maximum_size: 420,
                                set_hexpand: true,
                                gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_spacing: 16,
                                set_margin_all: 24,
                                set_valign: gtk::Align::Center,

                                gtk::Label {
                                    add_css_class: "title-1",
                                    set_wrap: true,
                                    set_justify: gtk::Justification::Center,
                                    #[watch]
                                    set_label: &model.track_name(),
                                },
                                gtk::Label {
                                    add_css_class: "dim-label",
                                    #[watch]
                                    set_label: &model.track_detail(),
                                },

                                gtk::ProgressBar {
                                    #[watch]
                                    set_fraction: model.state.track_fraction(),
                                },
                                gtk::Label {
                                    add_css_class: "numeric",
                                    add_css_class: "dim-label",
                                    #[watch]
                                    set_label: &model.position_label(),
                                },

                                // Reactions. The icon names here were
                                // checked against the installed theme:
                                // Adwaita has no thumbs-down glyph, so
                                // dislike uses its "not wanted" symbol.
                                gtk::Box {
                                    set_halign: gtk::Align::Center,
                                    set_spacing: 18,
                                    set_margin_top: 4,

                                    gtk::Button {
                                        add_css_class: "flat",
                                        add_css_class: "circular",
                                        set_tooltip_text: Some("Don't play this again"),
                                        set_icon_name: "action-unavailable-symbolic",
                                        #[watch]
                                        set_sensitive: model.state.track.is_some(),
                                        connect_clicked => Msg::Dislike,
                                    },
                                    gtk::Button {
                                        add_css_class: "flat",
                                        add_css_class: "circular",
                                        #[watch]
                                        set_icon_name: model.favorite_icon(),
                                        #[watch]
                                        set_css_classes: model.favorite_classes(),
                                        #[watch]
                                        set_tooltip_text: Some(model.favorite_tooltip()),
                                        #[watch]
                                        set_sensitive: model.state.track.is_some(),
                                        connect_clicked => Msg::ToggleFavorite,
                                    },
                                    gtk::Button {
                                        add_css_class: "flat",
                                        add_css_class: "circular",
                                        set_icon_name: "share-symbolic",
                                        set_tooltip_text: Some("Copy track details"),
                                        #[watch]
                                        set_sensitive: model.state.track.is_some(),
                                        connect_clicked => Msg::Share,
                                    },
                                },

                                gtk::Box {
                                    set_halign: gtk::Align::Center,
                                    set_spacing: 12,
                                    set_margin_top: 8,

                                    gtk::Button {
                                        set_icon_name: "media-playback-stop-symbolic",
                                        add_css_class: "circular",
                                        set_tooltip_text: Some("Stop"),
                                        connect_clicked => Msg::Stop,
                                    },
                                    gtk::Button {
                                        add_css_class: "circular",
                                        add_css_class: "suggested-action",
                                        set_width_request: 56,
                                        set_height_request: 56,
                                        #[watch]
                                        set_icon_name: model.play_icon(),
                                        connect_clicked => Msg::PlayPause,
                                    },
                                    gtk::Button {
                                        set_icon_name: "media-skip-forward-symbolic",
                                        add_css_class: "circular",
                                        set_tooltip_text: Some("Next track"),
                                        connect_clicked => Msg::Skip,
                                    },
                                },

                                gtk::Box {
                                    set_spacing: 8,
                                    set_margin_top: 8,
                                    gtk::Image { set_icon_name: Some("audio-volume-low-symbolic") },
                                    #[name = "volume"]
                                    gtk::Scale {
                                        set_hexpand: true,
                                        set_range: (0.0, 1.0),
                                        set_draw_value: false,
                                    },
                                    gtk::Image { set_icon_name: Some("audio-volume-high-symbolic") },
                                },

                                adw::PreferencesGroup {
                                    set_margin_top: 8,
                                    adw::ActionRow {
                                        set_title: "Session length",
                                        #[watch]
                                        set_subtitle: &model.timer_label(),
                                        set_activatable: true,
                                        add_suffix = &gtk::Image {
                                            set_icon_name: Some("go-next-symbolic"),
                                        },
                                        connect_activated => Msg::OpenModeDialog,
                                    },
                                },
                                },
                                },
                            },
                        },
                    },
                },
            }
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let client = Client::spawn();

        // Forward daemon updates onto the GTK loop. This is the whole
        // bridge: the bus runs on its own thread and runtime, and
        // `spawn_local` consumes the channel from inside GTK's main context,
        // so no widget is ever touched off-thread.
        {
            let updates = client.updates();
            let sender = sender.clone();
            relm4::spawn_local(async move {
                while let Ok(u) = updates.recv().await {
                    sender.input(Msg::FromDaemon(Box::new(u)));
                }
                tracing::warn!("update stream ended");
            });
        }

        let model = App {
            client,
            window: root.clone(),
            mode: SessionMode::default(),
            filters: Filters::default(),
            pending: None,
            catalog: None,
            notice: None,
            state: State::default(),
            connected: false,
            error: None,
            stack_ready: false,
        };

        // Loaded once. The gradient is heavier at the bottom, where the
        // controls sit, and lighter at the top where the title is.
        relm4::set_global_css(
            "
            .bfm-scrim {
                background: linear-gradient(
                    to bottom,
                    alpha(black, 0.45) 0%,
                    alpha(black, 0.72) 55%,
                    alpha(black, 0.88) 100%
                );
            }
            .bfm-scrim label { color: #ffffff; }
            .bfm-scrim label.dim-label { color: alpha(#ffffff, 0.72); }
            .bfm-faved { color: #e01b24; }
            ",
        );

        let widgets = view_output!();

        for (slug, title, subtitle) in STATES {
            let row = adw::ActionRow::builder()
                .title(title)
                .subtitle(subtitle)
                .activatable(true)
                .build();
            row.add_suffix(&gtk::Image::from_icon_name("media-playback-start-symbolic"));
            let s = sender.clone();
            row.connect_activated(move |_| s.input(Msg::ConfigureStart(slug)));
            widgets.state_rows.add(&row);
        }

        {
            let s = sender.clone();
            widgets
                .volume
                .connect_value_changed(move |sc| s.input(Msg::Volume(sc.value())));
        }

        if init.layer {
            setup_layer_shell(&root);
        }

        let mut model = model;
        model.stack_ready = true;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            Msg::FromDaemon(u) => match *u {
                Update::State(s) => {
                    self.connected = true;
                    self.state = *s;
                    if self.state.authenticated {
                        self.error = None;
                    }
                }
                Update::Disconnected(why) => {
                    self.connected = false;
                    self.error = Some(format!("Cannot reach the Brain.fm daemon: {why}"));
                }
                Update::Failed { code, message } => {
                    // "Sign in" is a screen, not an error banner.
                    self.error = if needs_login(&code) {
                        None
                    } else {
                        Some(message)
                    };
                }
                Update::Catalog(catalog) => {
                    self.catalog = Some((*catalog).clone());
                    match self.pending.take() {
                        // A stale reply must not open a wizard for a state
                        // the user is no longer choosing.
                        Some(Pending::Start(state)) if catalog.state == state => {
                            wizard::present_start_wizard(&self.window, state, &catalog, &sender);
                        }
                        Some(Pending::Length) => {
                            wizard::present_length_wizard(&self.window, &catalog, &sender);
                        }
                        _ => {}
                    }
                }
            },

            Msg::ConfigureStart(state) => {
                // Genres and activities differ per state, so a fresh set of
                // choices rather than whatever was picked for another mode.
                self.filters = Filters::default();
                self.pending = Some(Pending::Start(state));
                self.client.send(Command::FetchCatalog(state.to_owned()));
            }

            Msg::StartConfigured {
                state,
                filters,
                mode,
            } => {
                self.filters = *filters;
                self.mode = mode;
                self.client.send(Command::StartSession {
                    state: state.to_owned(),
                    activity: self.filters.activity.clone(),
                    genres: self.filters.genres.clone(),
                    effort: self.filters.effort.as_wire().to_owned(),
                    timer: mode.to_spec(),
                });
            }
            Msg::PlayPause => self.client.send(if self.state.playback.toggles_to_play() {
                Command::Resume
            } else {
                Command::Pause
            }),
            Msg::Skip => self.client.send(Command::Skip),
            Msg::Stop => self.client.send(Command::Stop),
            Msg::Volume(v) => self.client.send(Command::SetVolume(v)),
            Msg::DismissBanner => {
                self.error = None;
                self.notice = None;
            }

            Msg::ToggleFavorite => {
                let on = !self.state.track.as_ref().is_some_and(|t| t.is_favorite);
                self.client.send(Command::Favorite(on));
            }

            Msg::Dislike => self.client.send(Command::Dislike),

            Msg::Share => {
                // Brain.fm has no per-track URL, so this is a copy rather
                // than a link someone can follow to the track itself.
                if let Some(track) = self.state.track.as_ref() {
                    let text = share_text(track, &self.state.mental_state);
                    self.window.clipboard().set_text(&text);
                    self.notice = Some("Track details copied".into());
                }
            }

            Msg::OpenModeDialog => {
                // Reuse the catalog when we have one; its suggested lengths
                // are state-specific and worth keeping.
                match self.catalog.clone() {
                    Some(catalog) => wizard::present_length_wizard(&self.window, &catalog, &sender),
                    None => {
                        self.pending = Some(Pending::Length);
                        let state = if self.state.mental_state.is_empty() {
                            "focus".to_owned()
                        } else {
                            self.state.mental_state.clone()
                        };
                        self.client.send(Command::FetchCatalog(state));
                    }
                }
            }

            Msg::SetMode(mode) => {
                self.mode = mode;
                // If something is already playing, the choice should take
                // effect now rather than at the next session.
                if self.state.playback != Playback::Idle {
                    self.client.send(Command::SetTimer(mode.to_spec()));
                }
            }

            Msg::SignIn => match bfm_ui_core::find_session() {
                Some(found) => {
                    tracing::info!(profile = %found.profile, "adopting a browser session");
                    self.client
                        .send(Command::LoginFromBrowserToken(found.token));
                }
                None => {
                    self.error = Some(
                        "No Brain.fm session found in a local browser. Sign in at \
                         my.brain.fm, close the tab so the browser writes it to disk, \
                         then try again."
                            .into(),
                    );
                }
            },
        }
    }
}

impl App {
    fn banner(&self) -> Option<(bool, String)> {
        view::banner(
            self.state
                .daemon_is_stale()
                .then(|| self.state.stale_daemon_message()),
            self.error.as_deref(),
            self.notice.as_deref(),
        )
    }

    fn banner_text(&self) -> String {
        self.banner().map(|(_, t)| t).unwrap_or_default()
    }

    fn banner_is_error(&self) -> bool {
        self.banner().map(|(e, _)| e).unwrap_or(false)
    }

    fn has_banner(&self) -> bool {
        self.banner().is_some()
    }

    fn is_favorite(&self) -> bool {
        self.state.track.as_ref().is_some_and(|t| t.is_favorite)
    }

    fn favorite_icon(&self) -> &'static str {
        view::favorite_icon(self.is_favorite())
    }

    fn favorite_classes(&self) -> &'static [&'static str] {
        view::favorite_classes(self.is_favorite())
    }

    fn favorite_tooltip(&self) -> &'static str {
        view::favorite_tooltip(self.is_favorite())
    }

    fn scrim_classes(&self) -> &'static [&'static str] {
        view::scrim_classes(self.state.art_file().is_some())
    }

    /// `None` on the very first pass, before the stack has its children.
    fn page_if_ready(&self) -> Option<&'static str> {
        self.stack_ready.then(|| self.page())
    }

    fn page(&self) -> &'static str {
        view::page(
            self.connected,
            self.state.authenticated,
            self.state.playback,
        )
    }

    fn subtitle(&self) -> String {
        match self.page() {
            "player" => view::title_case(&self.state.mental_state),
            _ => String::new(),
        }
    }

    fn track_name(&self) -> String {
        view::track_name(self.state.track.as_ref())
    }

    fn track_detail(&self) -> String {
        view::track_detail(self.state.track.as_ref(), self.state.playback)
    }

    fn position_label(&self) -> String {
        let len = self.state.track.as_ref().map(|t| t.length).unwrap_or(0);
        view::position_label(self.state.position, len)
    }

    fn play_icon(&self) -> &'static str {
        view::play_icon(self.state.playback)
    }

    fn timer_label(&self) -> String {
        view::timer_label(&self.state.timer, self.mode)
    }
}

/// Anchor the window as a Wayland overlay.
///
/// The Hyprland-shaped way to use this: bind a key to `bfm-gtk --layer` and
/// get a HUD rather than a window to manage.
fn setup_layer_shell(window: &adw::ApplicationWindow) {
    use gtk4_layer_shell::{Edge, Layer, LayerShell};

    if !gtk4_layer_shell::is_supported() {
        tracing::warn!("compositor does not support layer-shell; showing a normal window");
        return;
    }
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("bfm"));
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top, 12);
    window.set_margin(Edge::Right, 12);
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::OnDemand);
}
