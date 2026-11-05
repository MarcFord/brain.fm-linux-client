//! The window.

use bfm_ui_core::{
    Catalog, Client, Command, Effort, Filters, Playback, SessionMode, State, Update,
    format_duration, needs_login, share_text,
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
    /// The state whose catalog we are waiting on, if any.
    pending_start: Option<&'static str>,
    /// A short confirmation, distinct from an error.
    notice: Option<String>,
    /// Set once the view has been built and the stack has its pages.
    stack_ready: bool,
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
                        set_reveal_child: model.error.is_some() || model.notice.is_some(),
                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 8,
                            set_margin_all: 8,
                            add_css_class: "card",

                            gtk::Image {
                                #[watch]
                                set_icon_name: Some(if model.error.is_some() {
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
                                set_label: model.banner_text(),
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
            pending_start: None,
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
                    // Only act on the catalog we actually asked for; a
                    // stale reply must not pop a dialog for another state.
                    if let Some(state) = self.pending_start.take()
                        && catalog.state == state
                    {
                        present_start_dialog(
                            &self.window,
                            state,
                            &catalog,
                            &self.filters,
                            self.mode,
                            &sender,
                        );
                    }
                }
            },

            Msg::ConfigureStart(state) => {
                // Genres and activities differ per state, so a fresh set of
                // choices rather than whatever was picked for another mode.
                self.filters = Filters::default();
                self.pending_start = Some(state);
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

            Msg::OpenModeDialog => present_mode_dialog(&self.window, self.mode, &sender),

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
    fn banner_text(&self) -> &str {
        self.error
            .as_deref()
            .or(self.notice.as_deref())
            .unwrap_or("")
    }

    fn is_favorite(&self) -> bool {
        self.state.track.as_ref().is_some_and(|t| t.is_favorite)
    }

    /// Adwaita has no outline heart, so the unfavourited state uses the
    /// emblem and relies on colour for the difference.
    fn favorite_icon(&self) -> &'static str {
        if self.is_favorite() {
            "heart-filled-symbolic"
        } else {
            "emblem-favorite-symbolic"
        }
    }

    fn favorite_classes(&self) -> &'static [&'static str] {
        if self.is_favorite() {
            &["flat", "circular", "bfm-faved"]
        } else {
            &["flat", "circular"]
        }
    }

    fn favorite_tooltip(&self) -> &'static str {
        if self.is_favorite() {
            "Remove from favourites"
        } else {
            "Add to favourites"
        }
    }

    /// Only dim when there is actually an image behind the controls.
    fn scrim_classes(&self) -> &'static [&'static str] {
        if self.state.art_file().is_some() {
            &["bfm-scrim"]
        } else {
            &[]
        }
    }

    /// `None` on the very first pass, before the stack has its children.
    fn page_if_ready(&self) -> Option<&'static str> {
        self.stack_ready.then(|| self.page())
    }

    fn page(&self) -> &'static str {
        if !self.connected {
            "connecting"
        } else if !self.state.authenticated {
            "signin"
        } else if self.state.playback == Playback::Idle {
            "browse"
        } else {
            "player"
        }
    }

    fn subtitle(&self) -> String {
        match self.page() {
            "player" => {
                let s = &self.state.mental_state;
                if s.is_empty() {
                    String::new()
                } else {
                    title_case(s)
                }
            }
            _ => String::new(),
        }
    }

    fn track_name(&self) -> String {
        self.state
            .track
            .as_ref()
            .map(|t| t.name.clone())
            .unwrap_or_else(|| bfm_ui_core::IDLE_LABEL.to_owned())
    }

    fn track_detail(&self) -> String {
        match (&self.state.track, self.state.playback) {
            (_, Playback::Buffering) => "Buffering…".to_owned(),
            (Some(t), _) if t.neural_effect > 0.0 => {
                format!("Neural effect {:.0}%", t.neural_effect * 100.0)
            }
            _ => String::new(),
        }
    }

    fn position_label(&self) -> String {
        let len = self.state.track.as_ref().map(|t| t.length).unwrap_or(0);
        format!(
            "{} / {}",
            format_duration(self.state.position),
            format_duration(len)
        )
    }

    fn play_icon(&self) -> &'static str {
        if self.state.playback.toggles_to_play() {
            "media-playback-start-symbolic"
        } else {
            "media-playback-pause-symbolic"
        }
    }

    fn timer_label(&self) -> String {
        let t = &self.state.timer;
        if !t.is_running() {
            return format!("{} · running until stopped", self.mode);
        }
        let phase = title_case(&t.phase);
        if t.cycles > 1 {
            format!(
                "{phase} · {} left · {}/{}",
                format_duration(t.remaining),
                t.cycle,
                t.cycles
            )
        } else {
            format!("{phase} · {} left", format_duration(t.remaining))
        }
    }
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
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

/// The session-length editor.
///
/// Built imperatively rather than in `view!` because it is transient and its
/// rows depend on the selected mode. Reached from both the browse page and
/// the player, so the control means the same thing wherever you are.
fn present_mode_dialog(
    parent: &adw::ApplicationWindow,
    current: SessionMode,
    sender: &ComponentSender<App>,
) {
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder()
        .title("Session length")
        .description("Infinite plays until you stop it.")
        .build();

    let modes = gtk::StringList::new(&["Infinite", "Timer", "Intervals"]);
    let mode_row = adw::ComboRow::builder()
        .title("Mode")
        .model(&modes)
        .selected(match current {
            SessionMode::Infinite => 0,
            SessionMode::Timer { .. } => 1,
            SessionMode::Intervals { .. } => 2,
        })
        .build();
    group.add(&mode_row);

    let minutes = adw::SpinRow::with_range(5.0, 600.0, 5.0);
    minutes.set_title("Minutes");
    minutes.set_value(match current {
        SessionMode::Timer { minutes } => minutes as f64,
        _ => 50.0,
    });
    group.add(&minutes);

    let work = adw::SpinRow::with_range(1.0, 180.0, 5.0);
    work.set_title("Work");
    work.set_subtitle("Minutes of focus per cycle");
    let rest = adw::SpinRow::with_range(1.0, 60.0, 1.0);
    rest.set_title("Break");
    rest.set_subtitle("Minutes between cycles");
    let cycles = adw::SpinRow::with_range(1.0, 12.0, 1.0);
    cycles.set_title("Cycles");
    if let SessionMode::Intervals {
        work: w,
        rest: r,
        cycles: c,
    } = current
    {
        work.set_value(w as f64);
        rest.set_value(r as f64);
        cycles.set_value(c as f64);
    } else {
        work.set_value(25.0);
        rest.set_value(5.0);
        cycles.set_value(4.0);
    }
    for row in [&work, &rest, &cycles] {
        group.add(row);
    }
    page.add(&group);

    // Only the rows belonging to the chosen mode are shown; an infinite
    // session has nothing to configure.
    let sync = {
        let (minutes, work, rest, cycles) =
            (minutes.clone(), work.clone(), rest.clone(), cycles.clone());
        move |selected: u32| {
            minutes.set_visible(selected == 1);
            for row in [&work, &rest, &cycles] {
                row.set_visible(selected == 2);
            }
        }
    };
    sync(mode_row.selected());
    mode_row.connect_selected_notify({
        let sync = sync.clone();
        move |row| sync(row.selected())
    });

    let read = {
        let (mode_row, minutes, work, rest, cycles) = (
            mode_row.clone(),
            minutes.clone(),
            work.clone(),
            rest.clone(),
            cycles.clone(),
        );
        move || match mode_row.selected() {
            1 => SessionMode::Timer {
                minutes: minutes.value() as u32,
            },
            2 => SessionMode::Intervals {
                work: work.value() as u32,
                rest: rest.value() as u32,
                cycles: cycles.value() as u32,
            },
            _ => SessionMode::Infinite,
        }
    };

    let dialog = adw::PreferencesDialog::builder()
        .title("Session length")
        .build();
    dialog.add(&page);

    // Applied on close: a preferences dialog with an OK button is not the
    // GNOME idiom, and the choice is cheap to change.
    dialog.connect_closed({
        let sender = sender.clone();
        move |_| sender.input(Msg::SetMode(read()))
    });
    dialog.present(Some(parent));
}

/// Configure and start a session for one mental state.
///
/// Everything here is per-state: Focus offers ten base genres, Sleep offers
/// one, and the suggested lengths run from fifteen minutes to nine hours. A
/// single global filter panel would be wrong for every state at once, so the
/// editor is built from the catalog the daemon just returned.
fn present_start_dialog(
    parent: &adw::ApplicationWindow,
    state: &'static str,
    catalog: &Catalog,
    current: &Filters,
    mode: SessionMode,
    sender: &ComponentSender<App>,
) {
    let page = adw::PreferencesPage::new();

    // ---- what to play ----
    let what = adw::PreferencesGroup::builder()
        .title("What to play")
        .build();

    let names = catalog.activity_names();
    let activity_row = adw::ComboRow::builder()
        .title("Activity")
        .model(&gtk::StringList::new(&names))
        .selected(catalog.index_of_activity(&current.activity))
        .build();
    what.add(&activity_row);

    let effort_row = adw::ComboRow::builder()
        .title("Neural effect")
        .subtitle(current.effort.description())
        .model(&gtk::StringList::new(&Effort::ALL.map(|e| e.label())))
        .selected(current.effort.index())
        .build();
    // The level names mean nothing on their own, so explain the selection.
    effort_row.connect_selected_notify(|row| {
        row.set_subtitle(Effort::from_index(row.selected()).description());
    });
    what.add(&effort_row);
    page.add(&what);

    // ---- genres ----
    //
    // Selecting none is not the same as selecting all: none means the
    // service picks freely, which is usually what people want.
    let genre_group = adw::PreferencesGroup::builder()
        .title("Genres")
        .description("Leave all off to let Brain.fm choose.")
        .build();
    let mut genre_rows = Vec::new();
    for genre in &catalog.genres {
        let row = adw::SwitchRow::builder()
            .title(genre)
            .active(current.has_genre(genre))
            .build();
        genre_group.add(&row);
        genre_rows.push((genre.clone(), row));
    }
    if !catalog.genres.is_empty() {
        page.add(&genre_group);
    }

    // ---- how long ----
    let length = adw::PreferencesGroup::builder()
        .title("Session length")
        .build();

    let mode_row = adw::ComboRow::builder()
        .title("Mode")
        .model(&gtk::StringList::new(&["Infinite", "Timer", "Intervals"]))
        .selected(match mode {
            SessionMode::Infinite => 0,
            SessionMode::Timer { .. } => 1,
            SessionMode::Intervals { .. } => 2,
        })
        .build();
    length.add(&mode_row);

    // Suggested lengths come from the service (interface version 2), with a
    // custom escape hatch.
    let suggested: Vec<String> = catalog
        .lengths
        .iter()
        .map(|(_, label)| label.clone())
        .chain(std::iter::once("Custom…".to_owned()))
        .collect();
    let suggested_refs: Vec<&str> = suggested.iter().map(String::as_str).collect();
    let custom_index = (suggested.len() - 1) as u32;

    let preset_row = adw::ComboRow::builder()
        .title("Length")
        .model(&gtk::StringList::new(&suggested_refs))
        .selected(custom_index)
        .build();
    length.add(&preset_row);

    let minutes = adw::SpinRow::with_range(5.0, 600.0, 5.0);
    minutes.set_title("Minutes");
    minutes.set_value(match mode {
        SessionMode::Timer { minutes } => minutes as f64,
        _ => 50.0,
    });
    length.add(&minutes);

    let work = adw::SpinRow::with_range(1.0, 180.0, 5.0);
    work.set_title("Work");
    let rest = adw::SpinRow::with_range(1.0, 60.0, 1.0);
    rest.set_title("Break");
    let cycles = adw::SpinRow::with_range(1.0, 12.0, 1.0);
    cycles.set_title("Cycles");
    if let SessionMode::Intervals {
        work: w,
        rest: r,
        cycles: c,
    } = mode
    {
        work.set_value(w as f64);
        rest.set_value(r as f64);
        cycles.set_value(c as f64);
    } else {
        work.set_value(25.0);
        rest.set_value(5.0);
        cycles.set_value(4.0);
    }
    for row in [&work, &rest, &cycles] {
        length.add(row);
    }
    page.add(&length);

    // Show only the rows the chosen mode actually uses.
    let sync = {
        let (preset_row, minutes, work, rest, cycles) = (
            preset_row.clone(),
            minutes.clone(),
            work.clone(),
            rest.clone(),
            cycles.clone(),
        );
        let has_presets = !catalog.lengths.is_empty();
        move |selected: u32| {
            let timer = selected == 1;
            preset_row.set_visible(timer && has_presets);
            minutes.set_visible(timer && (!has_presets || preset_row.selected() == custom_index));
            for row in [&work, &rest, &cycles] {
                row.set_visible(selected == 2);
            }
        }
    };
    sync(mode_row.selected());
    mode_row.connect_selected_notify({
        let sync = sync.clone();
        move |row| sync(row.selected())
    });
    preset_row.connect_selected_notify({
        let (sync, mode_row) = (sync.clone(), mode_row.clone());
        move |_| sync(mode_row.selected())
    });

    // ---- assemble ----
    let dialog = adw::Dialog::builder()
        .title(title_case(state))
        .content_width(420)
        .content_height(640)
        .build();

    let header = adw::HeaderBar::new();
    let start = gtk::Button::builder().label("Start").build();
    start.add_css_class("suggested-action");
    header.pack_end(&start);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_child(Some(&page));
    toolbar.set_content(Some(&scroller));
    dialog.set_child(Some(&toolbar));

    let lengths = catalog.lengths.clone();
    let order = catalog.genres.clone();
    let activities = catalog.activities.clone();

    start.connect_clicked({
        let dialog = dialog.clone();
        let sender = sender.clone();
        move |_| {
            let mut filters = Filters {
                activity: activities
                    .get(activity_row.selected() as usize)
                    .map(|(id, _)| id.clone())
                    .unwrap_or_default(),
                effort: Effort::from_index(effort_row.selected()),
                genres: Vec::new(),
            };
            for (genre, row) in &genre_rows {
                filters.toggle_genre(genre, row.is_active(), &order);
            }

            let mode = match mode_row.selected() {
                1 => {
                    let picked = preset_row.selected();
                    match lengths.get(picked as usize) {
                        Some((secs, _)) => SessionMode::Timer { minutes: secs / 60 },
                        None => SessionMode::Timer {
                            minutes: minutes.value() as u32,
                        },
                    }
                }
                2 => SessionMode::Intervals {
                    work: work.value() as u32,
                    rest: rest.value() as u32,
                    cycles: cycles.value() as u32,
                },
                _ => SessionMode::Infinite,
            };

            sender.input(Msg::StartConfigured {
                state,
                filters: Box::new(filters),
                mode,
            });
            dialog.close();
        }
    });

    dialog.present(Some(parent));
}
