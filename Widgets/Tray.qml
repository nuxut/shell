import QtQuick
import Quickshell
import Quickshell.Services.SystemTray
import Quickshell.Widgets
import "../Config"

Row {
  id: root
  spacing: 5
  height: parent.height
  property var barWindow: null

  property var activeMenu: null

  Repeater {
    model: SystemTray.items
    
    Item {
      width: root.height * 0.9 // Slightly wider for padding
      height: root.height * 0.9
      anchors.verticalCenter: parent.verticalCenter

      IconImage {
        anchors.centerIn: parent
        width: parent.width * 0.8
        height: parent.height * 0.8
        source: modelData.icon
        
        // Hover Effect: Darken (Opacity)
        opacity: mouseArea.containsMouse ? 0.7 : 1.0
        Behavior on opacity { NumberAnimation { duration: 150 } }
        
        MouseArea {
            id: mouseArea
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            acceptedButtons: Qt.LeftButton | Qt.RightButton
          
            onClicked: (mouse) => {
                if (mouse.button === Qt.LeftButton) {
                    modelData.activate();
                } else {
                    // Recreate menu to prevent stale state crashes
                    if (root.activeMenu) {
                        try {
                            root.activeMenu.close();
                            root.activeMenu.destroy();
                        } catch(e) {}
                        root.activeMenu = null;
                    }
                
                    if (modelData.menu) {
                        var comp = Qt.createComponent("TrayMenu.qml");
                        if (comp.status === Component.Ready) {
                            var popup = comp.createObject(root, {
                              parentWindow: root.barWindow
                            });
                            popup.externalHover = Qt.binding(function() { return mouseArea.containsMouse });
                            popup.open(modelData.menu, mouseArea); // Pass mouseArea as anchor
                            root.activeMenu = popup;
                        } else {
                            // console.log("Error loading TrayMenu:", comp.errorString());
                        }
                    } else {
                        modelData.secondaryActivate();
                    }
                }
            }
        }
      }
    }
  }
}
