mod bridge;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};

/// The module keeps the source directory in the resource path. Verified
/// against the resource names compiled into the binary, not inferred from
/// build output.
const MAIN_QML: &str = "qrc:/qt/qml/dev/marcford/bfm/qml/Main.qml";

/// Overridable, so a failing UI can be bisected without a rebuild.
fn qml_entry() -> String {
    std::env::var("BFM_QML").unwrap_or_else(|_| MAIN_QML.to_owned())
}

// Implemented in src/qt_log.cpp.
unsafe extern "C" {
    fn bfm_install_qt_message_handler();
    fn bfm_set_window_icon(name: *const std::ffi::c_char) -> bool;
}

/// Shared by both front-ends and both desktop entries.
const ICON_NAME: &str = "dev.marcford.bfm";

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("BFM_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .init();

    // Before anything Qt does, so nothing it reports is lost.
    unsafe { bfm_install_qt_message_handler() };

    let mut app = QGuiApplication::new();
    // Wayland derives app_id from this. Without it Qt falls back to the
    // binary name, so the window does not match the installed desktop
    // entry and gets neither its icon nor any compositor window rules.
    cxx_qt_lib::QGuiApplication::set_desktop_file_name(&QString::from("dev.marcford.bfm.kde"));
    // Registered after the application exists. Either order currently
    // behaves the same; this one matches Qt's own expectations.
    cxx_qt::init_qml_module!("dev.marcford.bfm");

    // After the application exists, before any window is shown.
    let icon = std::ffi::CString::new(ICON_NAME).expect("icon name");
    if !unsafe { bfm_set_window_icon(icon.as_ptr()) } {
        tracing::warn!(
            icon = ICON_NAME,
            "icon not found in the theme; install the package or set \
             XDG_DATA_DIRS so hicolor can see it"
        );
    }

    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from(&QString::from(&qml_entry())));
    }

    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
