import QtQuick
import QtQuick.Controls
import Quickshell
import "../Services"
import "../Config"

Item {
    id: root
    width: height
    // color: "transparent"

    Text {
        anchors.centerIn: parent
        text: AudioService.icon
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
        text: Math.round(AudioService.volume * 100) + "%"
    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        
        onClicked: {
            var cmd = "if pgrep pavucontrol > /dev/null; then pkill pavucontrol; else pavucontrol & fi";
            Quickshell.execDetached(["sh", "-c", cmd]);
        }
        
        onWheel: (wheel) => {
            if (wheel.angleDelta.y > 0) {
                AudioService.increaseVolume()
            } else {
                AudioService.decreaseVolume()
            }
        }
    }
}
