import QtQuick
import org.kde.kirigami as Kirigami

// Minimal bisection probe: if this shows a window and Main.qml does not,
// the fault is in our QML, not in module registration or the engine.
Kirigami.ApplicationWindow {
    visible: true
    width: 400
    height: 300
    title: "probe"
}
