//! One question per screen.
//!
//! The first cut put activity, effect, genres and length on a single
//! scrolling page with no obvious commit — four unrelated questions and no
//! clear way to finish. This asks one thing at a time: single-choice
//! questions advance the moment you tap an answer, and only the steps that
//! genuinely cannot (multi-select genres, numeric intervals) carry a button.
//!
//! `AdwNavigationView` provides Back for free, so a wrong turn costs one tap.
//! The sequencing lives in [`crate::steps`], where it is testable.

use std::{cell::RefCell, rc::Rc};

use bfm_ui_core::{
    Catalog, Effort, Filters, SessionMode,
    steps::{self, Step},
};
use relm4::{
    ComponentSender,
    adw::{self, prelude::*},
    gtk,
};

use crate::app::{App, Msg};

/// Answers collected so far.
#[derive(Default)]
struct Draft {
    filters: Filters,
    mode: SessionMode,
}

type Shared = Rc<RefCell<Draft>>;

/// A page: heading, explanation, the question itself, and at most one button.
fn page(
    step: Step,
    body: &impl IsA<gtk::Widget>,
    action: Option<&gtk::Button>,
) -> adw::NavigationPage {
    // A step either advances when tapped or carries a button. Neither means
    // a dead end; both means two ways to answer the same question.
    debug_assert_eq!(
        action.is_some(),
        !step.advances_on_tap(),
        "{step:?}: a step must advance on tap XOR carry a button"
    );
    let heading = gtk::Label::builder()
        .label(step.title())
        .css_classes(["title-2"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    let sub = gtk::Label::builder()
        .label(step.subtitle())
        .css_classes(["dim-label"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .margin_bottom(6)
        .build();

    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(8)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    column.append(&heading);
    column.append(&sub);
    column.append(body);
    if let Some(button) = action {
        button.set_margin_top(8);
        column.append(button);
    }

    // Only the question scrolls, so the heading and the button stay put
    // however long the list of genres is.
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(
            &adw::Clamp::builder()
                .maximum_size(400)
                .child(&column)
                .build(),
        )
        .build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scroller));

    adw::NavigationPage::builder()
        .title(step.title())
        .child(&toolbar)
        .build()
}

fn group() -> adw::PreferencesGroup {
    adw::PreferencesGroup::new()
}

fn chooser(title: &str, subtitle: Option<&str>) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .activatable(true)
        .build();
    if let Some(s) = subtitle {
        row.set_subtitle(s);
    }
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row
}

fn primary(label: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("pill");
    b.add_css_class("suggested-action");
    b.set_halign(gtk::Align::Center);
    b
}

/// Configure and start a session, one question at a time.
pub fn present_start_wizard(
    parent: &adw::ApplicationWindow,
    state: &'static str,
    catalog: &Catalog,
    sender: &ComponentSender<App>,
) {
    let draft: Shared = Rc::new(RefCell::new(Draft::default()));
    let nav = adw::NavigationView::new();

    let dialog = adw::Dialog::builder()
        .title(bfm_ui_core::title_case(state))
        .content_width(400)
        .content_height(560)
        .child(&nav)
        .build();

    // Sending the answers and closing is the one thing every terminal step
    // does, so it is defined once.
    let finish: Rc<dyn Fn()> = {
        let (draft, dialog, sender) = (draft.clone(), dialog.clone(), sender.clone());
        Rc::new(move || {
            let d = draft.borrow();
            sender.input(Msg::StartConfigured {
                state,
                filters: Box::new(d.filters.clone()),
                mode: d.mode,
            });
            dialog.close();
        })
    };

    nav.push(&activity_page(catalog, &draft, &nav, &finish));
    dialog.present(Some(parent));
}

fn activity_page(
    catalog: &Catalog,
    draft: &Shared,
    nav: &adw::NavigationView,
    finish: &Rc<dyn Fn()>,
) -> adw::NavigationPage {
    let list = group();
    for (id, name) in &catalog.activities {
        let row = chooser(name, None);
        let (id, draft, nav, finish, catalog) = (
            id.clone(),
            draft.clone(),
            nav.clone(),
            finish.clone(),
            catalog.clone(),
        );
        row.connect_activated(move |_| {
            draft.borrow_mut().filters.activity = id.clone();
            nav.push(&effort_page(&catalog, &draft, &nav, &finish));
        });
        list.add(&row);
    }
    page(Step::FIRST, &list, None)
}

fn effort_page(
    catalog: &Catalog,
    draft: &Shared,
    nav: &adw::NavigationView,
    finish: &Rc<dyn Fn()>,
) -> adw::NavigationPage {
    let list = group();
    for effort in Effort::ALL {
        let row = chooser(effort.label(), Some(effort.description()));
        let (draft, nav, finish, catalog) =
            (draft.clone(), nav.clone(), finish.clone(), catalog.clone());
        row.connect_activated(move |_| {
            draft.borrow_mut().filters.effort = effort;
            nav.push(&genres_page(&catalog, &draft, &nav, &finish));
        });
        list.add(&row);
    }
    page(Step::Effort, &list, None)
}

fn genres_page(
    catalog: &Catalog,
    draft: &Shared,
    nav: &adw::NavigationView,
    finish: &Rc<dyn Fn()>,
) -> adw::NavigationPage {
    let list = group();
    let mut rows = Vec::new();
    for genre in &catalog.genres {
        let row = adw::SwitchRow::builder()
            .title(genre)
            .active(draft.borrow().filters.has_genre(genre))
            .build();
        list.add(&row);
        rows.push((genre.clone(), row));
    }

    // Multi-select cannot advance on tap, so this one has a button.
    let button = primary(Step::Genres.continue_label().unwrap_or("Continue"));
    {
        let (draft, nav, finish, catalog) =
            (draft.clone(), nav.clone(), finish.clone(), catalog.clone());
        button.connect_clicked(move |_| {
            {
                let mut d = draft.borrow_mut();
                let order = catalog.genres.clone();
                for (genre, row) in &rows {
                    d.filters.toggle_genre(genre, row.is_active(), &order);
                }
            }
            nav.push(&length_kind_page(&catalog, &draft, &nav, &finish));
        });
    }
    page(Step::Genres, &list, Some(&button))
}

fn length_kind_page(
    catalog: &Catalog,
    draft: &Shared,
    nav: &adw::NavigationView,
    finish: &Rc<dyn Fn()>,
) -> adw::NavigationPage {
    let list = group();

    let choices = [
        ("Infinite", "Play until I stop it", SessionMode::Infinite),
        (
            "Timer",
            "End after a set time",
            SessionMode::Timer { minutes: 50 },
        ),
        (
            "Intervals",
            "Alternate focus and breaks",
            SessionMode::Intervals {
                work: 25,
                rest: 5,
                cycles: 4,
            },
        ),
    ];

    for (title, subtitle, mode) in choices {
        let row = chooser(title, Some(subtitle));
        let (draft, nav, finish, catalog) =
            (draft.clone(), nav.clone(), finish.clone(), catalog.clone());
        row.connect_activated(move |_| {
            draft.borrow_mut().mode = mode;
            match steps::next(Step::LengthKind, mode) {
                None => finish(),
                Some(Step::TimerLength) => {
                    nav.push(&timer_page(&catalog, &draft, &finish));
                }
                Some(Step::IntervalSetup) => {
                    nav.push(&interval_page(&draft, &finish));
                }
                Some(other) => tracing::error!(?other, "unexpected step after length"),
            }
        });
        list.add(&row);
    }
    page(Step::LengthKind, &list, None)
}

fn timer_page(catalog: &Catalog, draft: &Shared, finish: &Rc<dyn Fn()>) -> adw::NavigationPage {
    let list = group();

    // The service's own suggestions, which differ hugely by state — these
    // start immediately, since picking one is a complete answer.
    for (seconds, label) in &catalog.lengths {
        let row = chooser(label, None);
        let (minutes, draft, finish) = (seconds / 60, draft.clone(), finish.clone());
        row.connect_activated(move |_| {
            draft.borrow_mut().mode = SessionMode::Timer { minutes };
            finish();
        });
        list.add(&row);
    }

    let custom = adw::SpinRow::with_range(5.0, 600.0, 5.0);
    custom.set_title("Custom");
    custom.set_subtitle("Minutes");
    custom.set_value(50.0);
    list.add(&custom);

    let button = primary(Step::TimerLength.continue_label().unwrap_or("Start"));
    {
        let (draft, finish) = (draft.clone(), finish.clone());
        button.connect_clicked(move |_| {
            draft.borrow_mut().mode = SessionMode::Timer {
                minutes: custom.value() as u32,
            };
            finish();
        });
    }
    page(Step::TimerLength, &list, Some(&button))
}

fn interval_page(draft: &Shared, finish: &Rc<dyn Fn()>) -> adw::NavigationPage {
    let list = group();

    let work = adw::SpinRow::with_range(1.0, 180.0, 5.0);
    work.set_title("Focus");
    work.set_subtitle("Minutes");
    work.set_value(25.0);

    let rest = adw::SpinRow::with_range(1.0, 60.0, 1.0);
    rest.set_title("Break");
    rest.set_subtitle("Minutes");
    rest.set_value(5.0);

    let cycles = adw::SpinRow::with_range(1.0, 12.0, 1.0);
    cycles.set_title("Repeat");
    cycles.set_subtitle("How many focus periods");
    cycles.set_value(4.0);

    for row in [&work, &rest, &cycles] {
        list.add(row);
    }

    let button = primary(Step::IntervalSetup.continue_label().unwrap_or("Start"));
    {
        let (draft, finish) = (draft.clone(), finish.clone());
        button.connect_clicked(move |_| {
            draft.borrow_mut().mode = SessionMode::Intervals {
                work: work.value() as u32,
                rest: rest.value() as u32,
                cycles: cycles.value() as u32,
            };
            finish();
        });
    }
    page(Step::IntervalSetup, &list, Some(&button))
}

/// Change the length of a session that is already playing.
///
/// The same questions as the tail of the start wizard, but it applies
/// rather than starts — and unlike the old editor it says so on a button
/// instead of silently committing when the dialog closes.
pub fn present_length_wizard(
    parent: &adw::ApplicationWindow,
    catalog: &Catalog,
    sender: &ComponentSender<App>,
) {
    let draft: Shared = Rc::new(RefCell::new(Draft::default()));
    let nav = adw::NavigationView::new();
    let dialog = adw::Dialog::builder()
        .title("Session length")
        .content_width(400)
        .content_height(520)
        .child(&nav)
        .build();

    let finish: Rc<dyn Fn()> = {
        let (draft, dialog, sender) = (draft.clone(), dialog.clone(), sender.clone());
        Rc::new(move || {
            sender.input(Msg::SetMode(draft.borrow().mode));
            dialog.close();
        })
    };

    nav.push(&length_kind_page(catalog, &draft, &nav, &finish));
    dialog.present(Some(parent));
}
