import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import dev.marcford.bfm

Kirigami.ApplicationWindow {
    id: root
    title: "Brain.fm"
    width: Kirigami.Units.gridUnit * 24
    height: Kirigami.Units.gridUnit * 34
    minimumWidth: Kirigami.Units.gridUnit * 18

    Session {
        id: session
        Component.onCompleted: session.connectDaemon()
    }

    // Answers collected by the wizard, mirroring the GTK flow.
    QtObject {
        id: draft
        property string state: ""
        property int activityIndex: 0
        property string effort: "high"
        property var genres: []
        property int modeIndex: 0          // 0 infinite, 1 timer, 2 intervals
        property string timerSpec: ""
    }

    pageStack.initialPage: Kirigami.Page {
        title: session.mentalState.length > 0 ? session.mentalState : "Brain.fm"
        padding: 0

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            Kirigami.InlineMessage {
                Layout.fillWidth: true
                Layout.margins: Kirigami.Units.smallSpacing
                visible: session.bannerText.length > 0
                text: session.bannerText
                type: session.bannerIsError ? Kirigami.MessageType.Warning
                                            : Kirigami.MessageType.Positive
                showCloseButton: true
                onVisibleChanged: if (!visible) session.dismissBanner()
            }

            StackLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                currentIndex: {
                    switch (session.page) {
                    case "connecting": return 0
                    case "signin": return 1
                    case "browse": return 2
                    default: return 3
                    }
                }

                // ---- connecting ----
                Kirigami.PlaceholderMessage {
                    text: "Connecting"
                    explanation: "Starting the Brain.fm daemon…"
                    icon.name: "view-refresh"
                }

                // ---- signed out ----
                Kirigami.PlaceholderMessage {
                    text: "Sign in"
                    explanation: "Sign in to Brain.fm in your browser, then pick up the session here."
                    icon.name: "user-identity"
                    helpfulAction: Kirigami.Action {
                        text: "Use my browser session"
                        icon.name: "globe"
                        onTriggered: session.signIn()
                    }
                }

                // ---- choose a mode ----
                Controls.ScrollView {
                    contentWidth: availableWidth
                    ColumnLayout {
                        width: parent.width
                        spacing: 0

                        Repeater {
                            model: [
                                { slug: "focus",    name: "Focus",    hint: "Deep concentration" },
                                { slug: "relax",    name: "Relax",    hint: "Wind down" },
                                { slug: "sleep",    name: "Sleep",    hint: "Drift off" },
                                { slug: "meditate", name: "Meditate", hint: "Settle the mind" }
                            ]
                            delegate: Kirigami.BasicListItem {
                                required property var modelData
                                Layout.fillWidth: true
                                text: modelData.name
                                subtitle: modelData.hint
                                icon: "media-playback-start"
                                onClicked: {
                                    draft.state = modelData.slug
                                    draft.activityIndex = 0
                                    draft.effort = "high"
                                    draft.genres = []
                                    draft.modeIndex = 0
                                    draft.timerSpec = ""
                                    session.fetchCatalog(modelData.slug)
                                    root.pageStack.push(activityPage)
                                }
                            }
                        }
                    }
                }

                // ---- playing ----
                Item {
                    Image {
                        anchors.fill: parent
                        source: session.artPath
                        visible: session.artPath.length > 0
                        fillMode: Image.PreserveAspectCrop
                        asynchronous: true
                    }
                    // Artwork ranges from near-black to washed out, so
                    // legibility cannot be left to the image.
                    Rectangle {
                        anchors.fill: parent
                        visible: session.artPath.length > 0
                        gradient: Gradient {
                            GradientStop { position: 0.0; color: "#73000000" }
                            GradientStop { position: 1.0; color: "#e0000000" }
                        }
                    }

                    ColumnLayout {
                        anchors.centerIn: parent
                        width: Math.min(parent.width - Kirigami.Units.gridUnit * 2,
                                        Kirigami.Units.gridUnit * 22)
                        spacing: Kirigami.Units.largeSpacing

                        Kirigami.Heading {
                            Layout.fillWidth: true
                            level: 1
                            text: session.trackName
                            wrapMode: Text.WordWrap
                            horizontalAlignment: Text.AlignHCenter
                            color: session.artPath.length > 0 ? "white" : Kirigami.Theme.textColor
                        }
                        Controls.Label {
                            Layout.fillWidth: true
                            text: session.trackDetail
                            horizontalAlignment: Text.AlignHCenter
                            opacity: 0.75
                            color: session.artPath.length > 0 ? "white" : Kirigami.Theme.textColor
                        }

                        Controls.ProgressBar {
                            Layout.fillWidth: true
                            from: 0; to: 1
                            value: session.trackFraction
                        }
                        Controls.Label {
                            Layout.alignment: Qt.AlignHCenter
                            text: session.positionLabel
                            opacity: 0.75
                            color: session.artPath.length > 0 ? "white" : Kirigami.Theme.textColor
                        }

                        // Reactions
                        RowLayout {
                            Layout.alignment: Qt.AlignHCenter
                            spacing: Kirigami.Units.largeSpacing
                            Controls.ToolButton {
                                icon.name: "dialog-cancel"
                                display: Controls.AbstractButton.IconOnly
                                Controls.ToolTip.text: "Don't play this again"
                                Controls.ToolTip.visible: hovered
                                onClicked: session.dislike()
                            }
                            Controls.ToolButton {
                                icon.name: session.isFavorite ? "favorite" : "list-add"
                                display: Controls.AbstractButton.IconOnly
                                Controls.ToolTip.text: session.isFavorite ? "Remove from favourites"
                                                                          : "Add to favourites"
                                Controls.ToolTip.visible: hovered
                                onClicked: session.toggleFavorite()
                            }
                            Controls.ToolButton {
                                icon.name: "edit-copy"
                                display: Controls.AbstractButton.IconOnly
                                Controls.ToolTip.text: "Copy track details"
                                Controls.ToolTip.visible: hovered
                                onClicked: {
                                    // No per-track URL exists, so this copies
                                    // rather than linking. See docs/api.md.
                                    hiddenCopy.text = session.shareText()
                                    hiddenCopy.selectAll()
                                    hiddenCopy.copy()
                                }
                            }
                        }

                        // Transport
                        RowLayout {
                            Layout.alignment: Qt.AlignHCenter
                            spacing: Kirigami.Units.largeSpacing
                            Controls.ToolButton {
                                icon.name: "media-playback-stop"
                                onClicked: session.stop()
                            }
                            Controls.Button {
                                icon.name: session.page === "player" && session.trackDetail === "Buffering…"
                                           ? "media-playback-pause" : "media-playback-start"
                                text: ""
                                highlighted: true
                                onClicked: session.playPause()
                            }
                            Controls.ToolButton {
                                icon.name: "media-skip-forward"
                                onClicked: session.skip()
                            }
                        }

                        Controls.Slider {
                            Layout.fillWidth: true
                            from: 0; to: 1
                            value: session.volume
                            onMoved: session.applyVolume(value)
                        }

                        Kirigami.BasicListItem {
                            Layout.fillWidth: true
                            text: "Session length"
                            subtitle: session.timerLabel
                            icon: "clock"
                            onClicked: {
                                draft.state = ""
                                root.pageStack.push(lengthKindPage)
                            }
                        }
                    }
                }
            }
        }
    }

    // Off-screen helper: QtQuick has no clipboard API, but a TextEdit does.
    Controls.TextArea {
        id: hiddenCopy
        visible: false
    }

    // ---------------------------------------------------------- wizard

    Component {
        id: activityPage
        WizardPage {
            heading: "What are you doing?"
            explanation: "Pick the activity closest to your task."
            model: session.activityNames
            onChosen: function (index) {
                draft.activityIndex = index
                root.pageStack.push(effortPage)
            }
        }
    }

    Component {
        id: effortPage
        WizardPage {
            heading: "How strong?"
            explanation: "Neural effect strength. High is the default."
            model: ["Low", "Medium", "High"]
            subtitles: ["Gentlest — for sensitive listeners", "Balanced", "Strongest — the default"]
            onChosen: function (index) {
                draft.effort = ["low", "medium", "high"][index]
                root.pageStack.push(genresPage)
            }
        }
    }

    Component {
        id: genresPage
        Kirigami.ScrollablePage {
            title: "Any preference?"
            ColumnLayout {
                width: parent.width
                Kirigami.Heading { level: 2; text: "Any preference?" }
                Controls.Label {
                    text: "Leave everything off and Brain.fm will choose."
                    opacity: 0.7
                    Layout.bottomMargin: Kirigami.Units.largeSpacing
                }
                Repeater {
                    id: genreRepeater
                    model: session.genreNames
                    delegate: Controls.CheckBox {
                        required property string modelData
                        text: modelData
                    }
                }
                Controls.Button {
                    Layout.alignment: Qt.AlignHCenter
                    Layout.topMargin: Kirigami.Units.largeSpacing
                    text: "Continue"
                    highlighted: true
                    onClicked: {
                        let picked = []
                        for (let i = 0; i < genreRepeater.count; i++) {
                            const item = genreRepeater.itemAt(i)
                            if (item && item.checked) picked.push(item.text)
                        }
                        draft.genres = picked
                        root.pageStack.push(lengthKindPage)
                    }
                }
            }
        }
    }

    Component {
        id: lengthKindPage
        WizardPage {
            heading: "How long?"
            explanation: "Play until you stop, or set a timer."
            model: ["Infinite", "Timer", "Intervals"]
            subtitles: ["Play until I stop it", "End after a set time", "Alternate focus and breaks"]
            onChosen: function (index) {
                draft.modeIndex = index
                // The same sequencing the GTK wizard uses, decided in Rust.
                const nextStep = session.stepAfter(3, index)
                if (nextStep === -1) {
                    draft.timerSpec = ""
                    root.finish()
                } else if (nextStep === 4) {
                    root.pageStack.push(timerPage)
                } else {
                    root.pageStack.push(intervalPage)
                }
            }
        }
    }

    Component {
        id: timerPage
        Kirigami.ScrollablePage {
            title: "For how long?"
            ColumnLayout {
                width: parent.width
                Kirigami.Heading { level: 2; text: "For how long?" }
                Controls.Label {
                    text: "The session ends when the timer does."
                    opacity: 0.7
                    Layout.bottomMargin: Kirigami.Units.largeSpacing
                }
                Repeater {
                    model: session.lengthLabels
                    delegate: Kirigami.BasicListItem {
                        required property string modelData
                        required property int index
                        Layout.fillWidth: true
                        text: modelData
                        onClicked: {
                            const secs = session.lengthSeconds(index)
                            draft.timerSpec = Math.round(secs / 60) + "m"
                            root.finish()
                        }
                    }
                }
                RowLayout {
                    Layout.topMargin: Kirigami.Units.largeSpacing
                    Controls.Label { text: "Custom (minutes)" }
                    Controls.SpinBox {
                        id: customMinutes
                        from: 5; to: 600; stepSize: 5; value: 50
                        Layout.fillWidth: true
                    }
                }
                Controls.Button {
                    Layout.alignment: Qt.AlignHCenter
                    text: "Start"
                    highlighted: true
                    onClicked: {
                        draft.timerSpec = customMinutes.value + "m"
                        root.finish()
                    }
                }
            }
        }
    }

    Component {
        id: intervalPage
        Kirigami.ScrollablePage {
            title: "Work and break"
            ColumnLayout {
                width: parent.width
                Kirigami.Heading { level: 2; text: "Work and break" }
                Controls.Label {
                    text: "Alternating focus and rest, pomodoro style."
                    opacity: 0.7
                    Layout.bottomMargin: Kirigami.Units.largeSpacing
                }
                RowLayout {
                    Controls.Label { text: "Focus"; Layout.preferredWidth: Kirigami.Units.gridUnit * 5 }
                    Controls.SpinBox { id: workBox; from: 1; to: 180; stepSize: 5; value: 25; Layout.fillWidth: true }
                }
                RowLayout {
                    Controls.Label { text: "Break"; Layout.preferredWidth: Kirigami.Units.gridUnit * 5 }
                    Controls.SpinBox { id: restBox; from: 1; to: 60; value: 5; Layout.fillWidth: true }
                }
                RowLayout {
                    Controls.Label { text: "Repeat"; Layout.preferredWidth: Kirigami.Units.gridUnit * 5 }
                    Controls.SpinBox { id: cycleBox; from: 1; to: 12; value: 4; Layout.fillWidth: true }
                }
                Controls.Button {
                    Layout.alignment: Qt.AlignHCenter
                    Layout.topMargin: Kirigami.Units.largeSpacing
                    text: "Start"
                    highlighted: true
                    onClicked: {
                        draft.timerSpec = workBox.value + "m/" + restBox.value + "m"
                            + (cycleBox.value > 1 ? " x" + cycleBox.value : "")
                        root.finish()
                    }
                }
            }
        }
    }

    /// Apply the collected answers: start a session, or just change the
    /// timer when the wizard was opened mid-session.
    function finish() {
        if (draft.state.length > 0) {
            session.startConfigured(draft.state, draft.activityIndex, draft.effort,
                                    draft.genres.join("\n"), draft.timerSpec)
        } else {
            session.setTimerSpec(draft.timerSpec)
        }
        while (root.pageStack.depth > 1) root.pageStack.pop()
    }
}
