import QtQuick
import Quickshell
import "../Config"

PopupWindow {
    id: root
    property var parentWindow
    property var anchorItem
    property string text

    anchor.window: parentWindow
    anchor.item: anchorItem
    
    // Center horizontally relative to anchor
    // anchor.rect.x is offset relative to anchorItem's top-left
    anchor.rect.x: (anchorItem.width - width) / 2
    
    // Position below:
    // y offset = anchorItem.height + gap
    anchor.rect.y: anchorItem.height + 5
    
    implicitWidth: label.implicitWidth + 16
    implicitHeight: label.implicitHeight + 16
    
    color: "transparent"
    
    Rectangle {
        anchors.fill: parent
        color: Qt.lighter(Config.bar.color, 1.5)
        radius: Config.bar.radius
        
        Text {
            id: label
            anchors.centerIn: parent
            text: root.text
            color: Config.theme.text
            font.family: Config.font.family
            font.pixelSize: 13
        }
    }
}
