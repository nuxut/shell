import QtQuick
import Quickshell
import Quickshell.Hyprland
import "../Config"

Item {
  id: root
  implicitWidth: content.implicitWidth
  implicitHeight: root.height

  Row {
    id: content
    anchors.centerIn: parent
    spacing: 5

    Repeater {
      model: Hyprland.workspaces.values
      
      Rectangle {
        readonly property bool isFocused: Hyprland.focusedWorkspace && modelData.id === Hyprland.focusedWorkspace.id
        width: isFocused ? 20 : 8
        height: 8
        radius: 4
        color: isFocused ? Config.theme.accent : Config.theme.inactive
        
        Behavior on width { NumberAnimation { duration: 200 } }

        MouseArea {
          anchors.fill: parent
          onClicked: {
              Quickshell.execDetached(["hyprctl", "dispatch", "workspace", modelData.name]);
          }
        }
      }
    }
  }
}
