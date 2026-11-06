use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("dev.marcford.bfm").qml_files([
        "qml/Main.qml",
        "qml/WizardPage.qml",
        "qml/Probe.qml",
    ]))
    .file("src/bridge.rs")
    .cpp_files(["src/qt_log.cpp", "src/qt_icon.cpp"])
    .qt_module("Quick")
    .build();
}
