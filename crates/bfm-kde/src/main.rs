mod bridge;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};

/// The module keeps the source directory in the resource path, so this
/// mirrors `qml_files` in build.rs. Verified against the resource names
/// compiled into the binary rather than inferred from build output.
const MAIN_QML: &str = "qrc:/qt/qml/dev/marcford/bfm/qml/Main.qml";

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("BFM_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .init();

    // Registers the QML module's types and its compiled .qml resources.
    cxx_qt::init_qml_module!("dev.marcford.bfm");

    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();

    if let Some(engine) = engine.as_mut() {
        // A failed load leaves the event loop running with no window, which
        // from outside is indistinguishable from working — so the smoke test
        // for this binary must assert a window exists, not that the process
        // survived.
        engine.load(&QUrl::from(&QString::from(MAIN_QML)));
    }

    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
