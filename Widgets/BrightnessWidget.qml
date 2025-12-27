import QtQuick
import QtQuick.Controls
import "../Services"
import "../Config"

Item {
    id: root
    width: height
    // color: "transparent"

    Text {
        anchors.centerIn: parent
        text: BrightnessService.icon
        color: Config.theme.text
        font.family: Config.font.family
        font.pixelSize: 16
        
        // Hover Effect: Darker (Opacity)
        opacity: mouseArea.containsMouse ? 0.7 : 1.0
        Behavior on opacity { NumberAnimation { duration: 150 } }
    }

    property var barWindow: null

    SimpleTooltip {
        visible: mouseArea.containsMouse
        parentWindow: root.barWindow
        anchorItem: mouseArea
        text: Math.round(BrightnessService.brightness * 100) + "%"
    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        
        onWheel: (wheel) => {
            if (wheel.angleDelta.y > 0) {
                BrightnessService.increaseBrightness()
            } else {
                BrightnessService.decreaseBrightness()
            }
        }
    }
}
