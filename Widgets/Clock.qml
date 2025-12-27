import QtQuick
import Quickshell
import "../Config"

Item {
  id: root
  implicitWidth: timeText.implicitWidth
  implicitHeight: root.height

  Text {
    id: timeText
    anchors.centerIn: parent
    text: {
        var date = new Date();
        return date.toLocaleTimeString(Qt.locale(), "HH:mm");
    }
    color: Config.theme.text
    font.family: Config.font.family
    font.pixelSize: Config.font.size
    
    // Hover Effect: Darken (Opacity)
    opacity: mouseArea.containsMouse ? 0.7 : 1.0
    Behavior on opacity { NumberAnimation { duration: 150 } }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: {
            var cmd = "if pgrep gnome-calendar > /dev/null; then pkill gnome-calendar; else gnome-calendar & fi";
            Quickshell.execDetached(["sh", "-c", cmd]);
        }
    }
  }

  Timer {
    interval: 1000
    running: true
    repeat: true
    onTriggered: timeText.text = new Date().toLocaleTimeString(Qt.locale(), "HH:mm")
  }
}
