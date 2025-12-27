import QtQuick
import Quickshell
import "../Config"

Item {
  id: root
  implicitWidth: iconText.implicitWidth + 10
  implicitHeight: root.height
  
  property var panel: null

  Text {
    id: iconText
    anchors.centerIn: parent
    text: "\uf303" // nf-linux-arch
    color: Config.theme.accent // Arch Blue -> Sapphire
    font.family: Config.font.family
    font.pixelSize: Config.font.size + 4
    MouseArea {
      anchors.fill: parent
      cursorShape: Qt.PointingHandCursor
      hoverEnabled: true
      
      onClicked: {
        if (root.panel) {
            root.panel.isOpen = !root.panel.isOpen
        }
      }
      
      onEntered: {
          parent.opacity = 0.7
          if (root.panel) root.panel.externalHover = true
      }
      onExited: {
          parent.opacity = 1.0
          if (root.panel) root.panel.externalHover = false
      }
    }
  }
}
