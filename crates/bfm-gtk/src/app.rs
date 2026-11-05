//! The window.

use bfm_ui_core::{
    Client, Command, Playback, SessionMode, State, Update, format_duration, needs_login,
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
    effort: &'static str,
    /// How long the next session should run. Applied immediately when one is
    /// already playing, so the control means the same thing on both pages.
    mode: SessionMode,
    /// Kept so a transient dialog has something to be presented on.
    window: adw::ApplicationWindow,
    /// Set once the view has been built and the stack has its pages.
    stack_ready: bool,
}

#[derive(Debug)]
pub enum Msg {
    FromDaemon(Box<Update>),
    Play(&'static str),
    PlayPause,
    Skip,
    Stop,
    SignIn,
    Volume(f64),
    OpenModeDialog,
    SetMode(SessionMode),
    DismissError,
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
                        set_reveal_child: model.error.is_some(),
                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 8,
                            set_margin_all: 8,
                            add_css_class: "card",

                            gtk::Image { set_icon_name: Some("dialog-warning-symbolic") },
                            gtk::Label {
                                set_wrap: true,
                                set_xalign: 0.0,
                                set_hexpand: true,
                                #[watch]
                                set_label: model.error.as_deref().unwrap_or(""),
                            },
                            gtk::Button {
                                set_icon_name: "window-close-symbolic",
                                add_css_class: "flat",
                                connect_clicked => Msg::DismissError,
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
            state: State::default(),
            connected: false,
            error: None,
            effort: "high",
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
            row.connect_activated(move |_| s.input(Msg::Play(slug)));
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
                Update::Catalog { .. } => {}
            },

            Msg::Play(state) => self.client.send(Command::StartSession {
                state: state.to_owned(),
                activity: String::new(),
                genres: Vec::new(),
                effort: self.effort.to_owned(),
                timer: self.mode.to_spec(),
            }),
            Msg::PlayPause => self.client.send(if self.state.playback.toggles_to_play() {
                Command::Resume
            } else {
                Command::Pause
            }),
            Msg::Skip => self.client.send(Command::Skip),
            Msg::Stop => self.client.send(Command::Stop),
            Msg::Volume(v) => self.client.send(Command::SetVolume(v)),
            Msg::DismissError => self.error = None,

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
