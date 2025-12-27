import Quickshell
import QtQuick
import "./Widgets"
import "./Services"

ShellRoot {
  Variants {
    model: Quickshell.screens
    
    delegate: Component {
      QtObject {
        id: delegateRoot
        required property var modelData

        property var bar: Bar {
          screen: delegateRoot.modelData
          notificationCenter: delegateRoot.center
          leftPanel: delegateRoot.leftPanel
        }

        property var popup: NotificationPopup {
          screen: delegateRoot.modelData
        }

        property var center: NotificationCenter {
          screen: delegateRoot.modelData
          externalHover: delegateRoot.bar.bellHovered
          service: NotificationService
          historyModel: NotificationService.historyList
        }
        
        property var leftPanel: LeftPanel {
             screen: delegateRoot.modelData
        }
      }
    }
  }
}
