import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import org.kde.kirigami.delegates as Delegates

/// One single-choice question. Tapping an answer advances immediately —
/// that is what makes the flow feel like clicking through rather than
/// filling in a form.
Kirigami.ScrollablePage {
    id: page
    property string heading: ""
    property string explanation: ""
    property var model: []
    property var subtitles: []
    signal chosen(int index)

    title: heading

    ColumnLayout {
        width: parent.width

        Kirigami.Heading { level: 2; text: page.heading }
        Controls.Label {
            text: page.explanation
            opacity: 0.7
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
            Layout.bottomMargin: Kirigami.Units.largeSpacing
        }

        Repeater {
            model: page.model
            delegate: Delegates.SubtitleDelegate {
                required property string modelData
                required property int index
                Layout.fillWidth: true
                text: modelData
                subtitle: index < page.subtitles.length ? page.subtitles[index] : ""
                icon.name: "go-next"
                onClicked: page.chosen(index)
            }
        }
    }
}
