mod bridge;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};

fn main() {
    // Registers the QML module's types and its compiled .qml resources.
    // Without this the engine finds nothing and exits immediately with no
    // error — a silent no-op rather than a failure.
    cxx_qt::init_qml_module!("dev.marcford.bfm");

    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from(&QString::from(
            "qrc:/qt/qml/dev/marcford/bfm/qml/Main.qml",
        )));
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
