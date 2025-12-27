import QtQuick
import QtQuick.Layouts
import QtQuick.Controls
import Quickshell
import Quickshell.Wayland
import "../Config"

PanelWindow {
    id: root
    
    property bool isOpen: false
    property bool externalHover: false
    property bool internalHover: false
    property bool clearBtnHovered: false
    
    property var service: null
    property ListModel historyModel: null
    
    Timer {
        interval: 500
        repeat: true
        running: isOpen
        onTriggered: {
            if (!internalHover && !externalHover && !clearBtnHovered) {
                isOpen = false;
            }
        }
    }

    anchors {
        top: true
        right: true
    }
    
    // Position it below the visual bar (30px)

    margins.right: 0

    implicitWidth: 400
    implicitHeight: 600
    
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.namespace: "nuxut-notification-center"
    WlrLayershell.exclusionMode: ExclusionMode.Ignore
    
    // Animation Logic
    property real showProgress: isOpen ? 100 : 0 // 0 to 100
    Behavior on showProgress { NumberAnimation { duration: 300; easing.type: Easing.OutCubic } }
    
    // Visible as long as it's partially shown
    visible: showProgress > 0
    
    // Animate position: 29 when open, -height when closed
    margins.top: 29 - (1.0 - (showProgress / 100.0)) * (root.implicitHeight + 50)
    
    color: "transparent"
    
    Rectangle {
        anchors.fill: parent
        color: Config.bar.color
        radius: Config.bar.radius
        
        MouseArea {
            anchors.fill: parent
            hoverEnabled: true
            onContainsMouseChanged: root.internalHover = containsMouse
        }

        // Square top corners to merge with bar
        Rectangle {
            height: parent.radius
            width: parent.width
            color: parent.color
            anchors.top: parent.top
        }
        
        ColumnLayout {
        anchors.fill: parent
        anchors.margins: 15
        spacing: 15
        
        // Header
        RowLayout {
            Layout.fillWidth: true
            
            Text {
                text: "Notifications"
                color: Config.theme.text
                font.pixelSize: 20
                font.family: Config.font.family
                font.bold: true
                Layout.fillWidth: true
            }
            
            Rectangle {
                Layout.preferredHeight: 30
                Layout.preferredWidth: 80
                color: clearMouseArea.containsMouse ? Qt.lighter(Config.theme.accent, 1.1) : Config.theme.accent
                radius: 6
                visible: historyModel && historyModel.count > 0
                
                Text {
                    anchors.centerIn: parent
                    text: "Clear All"
                    color: Config.bar.color
                    font.family: Config.font.family
                    font.bold: true
                    font.pixelSize: 12
                }
                
                MouseArea {
                    id: clearMouseArea
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onContainsMouseChanged: root.clearBtnHovered = containsMouse
                    onClicked: service.clearHistory()
                }
            }
        }
        
        // List
        ListView {
            id: historyView
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: historyModel
            spacing: 10
            
            delegate: Rectangle {
                width: ListView.view.width
                height: contentCol.height + 20
                color: Config.theme.inactive
                radius: 6
                
                ColumnLayout {
                    id: contentCol
                    anchors {
                        top: parent.top
                        left: parent.left
                        right: parent.right
                        margins: 10
                    }
                    spacing: 4
                    
                    RowLayout {
                        Text {
                            text: model.appName
                            font.family: Config.font.family
                            font.bold: true
                            color: Config.theme.text
                            opacity: 0.7
                            font.pixelSize: 11
                        }
                        Item { Layout.fillWidth: true }
                        Text {
                            text: model.id // timestamp might be better if we had it
                            color: "#666"
                            font.family: Config.font.family
                            font.pixelSize: 10
                            visible: false
                        }
                    }
                    
                    Text {
                        text: model.summary
                        font.family: Config.font.family
                        font.bold: true
                        color: Config.theme.text
                        font.pixelSize: 13
                        wrapMode: Text.Wrap
                        Layout.fillWidth: true
                    }
                    
                    Text {
                        text: model.body
                        color: Config.theme.text
                        opacity: 0.8
                        font.family: Config.font.family
                        font.pixelSize: 12
                        wrapMode: Text.Wrap
                        Layout.fillWidth: true
                        visible: text.length > 0
                    }
                }
                
                MouseArea {
                    anchors.fill: parent
                    // Maybe click to remove? or right click?
                    acceptedButtons: Qt.RightButton
                    onClicked: {
                        if (mouse.button === Qt.RightButton) {
                            service.removeHistoryItem(index)
                        }
                    }
                }
            }

            Text {
                anchors.centerIn: parent
                text: "No notifications"
                color: Config.theme.text
                opacity: 0.5
                visible: parent.count === 0
                font.family: Config.font.family
                font.pixelSize: 16
            }
        }
        }
    }
}
