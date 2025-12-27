import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import "../Services"
import "../Config"

PanelWindow {
    id: root
    
    // Position the notification window
    anchors {
        top: true
        right: true
    }
    margins {
        top: 10 // Leave space for a top bar if needed, or adjust as desired
        right: 10
    }

    implicitWidth: 400
    implicitHeight: Math.min(notifList.contentHeight, 1000)

    color: "transparent"
    
    // Set layer to Overlay to appear above most things
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.namespace: "nuxut-notifications"
    // Transparent background allows clicks to pass through to windows below
    
    ListView {
        id: notifList
        anchors.fill: parent
        anchors.bottom: undefined // Allow it to grow downwards
        height: Math.min(contentHeight, parent.height)
        
        spacing: 10
        model: NotificationService.activeList

        delegate: Rectangle {
            width: ListView.view.width
            height: contentCol.height + 20
            color: Config.theme.inactive // Dark background
            radius: 8
            border.color: "#444444"
            border.width: 1

            // Simple shadow effect (optional, keep simple for now)
            
            ColumnLayout {
                id: contentCol
                anchors {
                    top: parent.top
                    left: parent.left
                    right: parent.right
                    margins: 10
                }
                spacing: 5

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 10

                    // App Name
                    Text {
                        text: model.appName
                        font.family: Config.font.family
                        font.bold: true
                        color: Config.theme.text
                        font.pixelSize: 12
                    }
                    
                    Item { Layout.fillWidth: true }
                    
                    // Close button
                    Text {
                        text: "✕"
                        color: Config.theme.text
                        font.family: Config.font.family
                        font.pixelSize: 14
                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: NotificationService.dismissNotification(model.id)
                        }
                    }
                }

                Text {
                    text: model.summary
                    font.family: Config.font.family
                    font.bold: true
                    color: Config.theme.text
                    font.pixelSize: 14
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

                Text {
                    text: model.body
                    color: Config.theme.text
                    font.family: Config.font.family
                    font.pixelSize: 13
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                    visible: text.length > 0
                }
            }
            
            MouseArea {
                // Clicking the notification body could perform a default action or dismiss
                anchors.fill: parent
                z: -1 // Behind the close button
                onClicked: {
                   // For now just dismiss on click, or do nothing
                   NotificationService.dismissNotification(model.id)
                }
            }
        }
    }
}
