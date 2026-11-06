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

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("BFM_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let mut app = QGuiApplication::new();
    // Registered after the application exists. Either order currently
    // behaves the same; this one matches Qt's own expectations.
    cxx_qt::init_qml_module!("dev.marcford.bfm");

    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        // KNOWN BROKEN: this returns without error and without producing a
        // window. See "the KDE front-end does not display" in
        // docs/architecture.md for what has been ruled out.
        engine.load(&QUrl::from(&QString::from(&qml_entry())));
    }

    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
