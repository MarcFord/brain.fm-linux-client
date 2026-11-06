import QtQuick
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import dev.marcford.bfm

Kirigami.ApplicationWindow {
    id: root
    title: "Brain.fm"
    width: 420
    height: 560

    Session { id: session }

    pageStack.initialPage: Kirigami.Page {
        title: "Brain.fm"
        Controls.Label {
            anchors.centerIn: parent
            text: session.probe
        }
    }
}
