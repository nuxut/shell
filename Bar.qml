import Quickshell
import QtQuick
import QtQuick.Shapes
import QtQuick.Layouts
import Quickshell.Wayland
import "Helpers"
import "Config"
import "Widgets"
import "Services"
import QtQuick.Controls

PanelWindow {
  id: barWindow
  anchors {
    top: true
    left: true
    right: true
  }
  
  property var notificationCenter: null
  property var leftPanel: null
  property bool bellHovered: false
  
  // Visual height of the bar
  property real contentHeight: 30
  
  // Window needs to be taller than content to allow corners to extend downwards
  implicitHeight: contentHeight + radius + Config.bar.bottomGap
  
  // Only reserve space for the content + configurable gap
  exclusiveZone: contentHeight + Config.bar.bottomGap

  // Explicitly set transparent background using RGBA
  color: Qt.rgba(0, 0, 0, 0)

  // Mask input to only the content visual area.
  // The Region object supports declarative geometry bindings.
  mask: Region {
      x: 0
      y: 0
      width: barWindow.width
      height: contentHeight
  }



  // Define corner states. 
  // top: -1 (flat)
  // bottom: 2 (vertical invert - "bended downwards")
  property int topCornerState: -1
  property int bottomCornerState: 2 
  property real radius: Config.bar.radius
  property color bgColor: Config.bar.color

  Shape {
    id: backgroundShape
    anchors.fill: parent
    
    // Antialiasing for smooth curves
    layer.enabled: true
    layer.samples: 4

    ShapePath {
      id: path
      strokeWidth: 0
      strokeColor: "transparent"
      fillColor: bgColor

      // Calculate radii and multipliers
      readonly property real w: backgroundShape.width
      // H is the visual height, not the full window height
      readonly property real h: contentHeight
      
      // Flatten radius if needed
      readonly property bool flatten: ShapeCornerHelper.shouldFlatten(w, h, radius)
      readonly property real r: flatten ? ShapeCornerHelper.getFlattenedRadius(Math.min(w, h), radius) : radius

      // Corner Multipliers & Radii
      readonly property real tlMultX: ShapeCornerHelper.getMultX(topCornerState)
      readonly property real tlMultY: ShapeCornerHelper.getMultY(topCornerState)
      readonly property real tlR: (topCornerState === -1) ? 0 : r

      readonly property real trMultX: ShapeCornerHelper.getMultX(topCornerState)
      readonly property real trMultY: ShapeCornerHelper.getMultY(topCornerState)
      readonly property real trR: (topCornerState === -1) ? 0 : r

      readonly property real brMultX: ShapeCornerHelper.getMultX(bottomCornerState)
      readonly property real brMultY: ShapeCornerHelper.getMultY(bottomCornerState)
      readonly property real brR: (bottomCornerState === -1) ? 0 : r

      readonly property real blMultX: ShapeCornerHelper.getMultX(bottomCornerState)
      readonly property real blMultY: ShapeCornerHelper.getMultY(bottomCornerState)
      readonly property real blR: (bottomCornerState === -1) ? 0 : r

      startX: path.tlR * path.tlMultX
      startY: 0

      // Top Edge & TR Corner
      PathLine { relativeX: path.w - path.tlR * path.tlMultX - path.trR * path.trMultX; relativeY: 0 }
      PathArc {
        relativeX: path.trR * path.trMultX; relativeY: path.trR * path.trMultY
        radiusX: path.trR; radiusY: path.trR
        direction: ShapeCornerHelper.getArcDirection(path.trMultX, path.trMultY)
      }

      // Right Edge & BR Corner
      PathLine { relativeX: 0; relativeY: path.h - path.trR * path.trMultY - path.brR * path.brMultY }
      PathArc {
        relativeX: -path.brR * path.brMultX; relativeY: path.brR * path.brMultY
        radiusX: path.brR; radiusY: path.brR
        direction: ShapeCornerHelper.getArcDirection(path.brMultX, path.brMultY)
      }

      // Bottom Edge & BL Corner
      PathLine { relativeX: -(path.w - path.brR * path.brMultX - path.blR * path.blMultX); relativeY: 0 }
      PathArc {
        relativeX: -path.blR * path.blMultX; relativeY: -path.blR * path.blMultY
        radiusX: path.blR; radiusY: path.blR
        direction: ShapeCornerHelper.getArcDirection(path.blMultX, path.blMultY)
      }

      // Left Edge & TL Corner
      PathLine { relativeX: 0; relativeY: -(path.h - path.blR * path.blMultY - path.tlR * path.tlMultY) }
      PathArc {
        relativeX: path.tlR * path.tlMultX; relativeY: -path.tlR * path.tlMultY
        radiusX: path.tlR; radiusY: path.tlR
        direction: ShapeCornerHelper.getArcDirection(path.tlMultX, path.tlMultY)
      }
    }
  }

  // Wrapper item to ensure text is centered within the contentHeight
  Item {
    width: parent.width
    height: contentHeight
    anchors.top: parent.top
    
    // Left Section
    Row {
      anchors.left: parent.left
      anchors.leftMargin: 5
      anchors.verticalCenter: parent.verticalCenter
      spacing: 5
      ArchButton { 
          height: contentHeight
          panel: leftPanel
      }
      SysInfo { height: contentHeight }
    }

    // Center Section
    Workspaces {
      anchors.centerIn: parent
      height: contentHeight
    }

    // Right Section
    Row {
      anchors.right: parent.right
      anchors.rightMargin: 15
      anchors.verticalCenter: parent.verticalCenter
      spacing: 5
      
      Tray { height: contentHeight; barWindow: barWindow }
      
      BrightnessWidget { 
          height: contentHeight
          barWindow: barWindow
      }
      
      VolumeWidget { 
          height: contentHeight 
          barWindow: barWindow
      }
      
      // Notification Center Toggle
      Rectangle {
        id: notifToggle
        height: contentHeight
        width: height
        color: "transparent"
        
        property int historyCount: NotificationService.historyList.count
        property int activeCount: NotificationService.activeList.count
        
        opacity: notifMouseArea.containsMouse ? 0.7 : 1.0
        Behavior on opacity { NumberAnimation { duration: 150 } }
        
        Text {
            anchors.centerIn: parent
            // Logic:
            // Active Popup -> Ringing (\udb80\udc9e)
            // History > 0 -> Notified (\udb84\udd6b)
            // Else -> Idle (\udb80\udc9a)
            text: notifToggle.activeCount > 0 ? "\udb80\udc9e" : (notifToggle.historyCount > 0 ? "\udb84\udd6b" : "\udb80\udc9a")
            color: Config.theme.text
            font.family: Config.font.family
            font.pixelSize: 16
        }
        
        SimpleTooltip {
             visible: notifMouseArea.containsMouse
             parentWindow: barWindow
             anchorItem: notifMouseArea
             text: notifToggle.historyCount === 0 ? "No notifications" : (notifToggle.historyCount + " notification" + (notifToggle.historyCount > 1 ? "s" : ""))
        }
        
        MouseArea {
            id: notifMouseArea
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onContainsMouseChanged: barWindow.bellHovered = containsMouse
            onClicked: {
                if (barWindow.notificationCenter) {
                    barWindow.notificationCenter.isOpen = !barWindow.notificationCenter.isOpen
                }
            }
        }
      }
      


      Clock { height: contentHeight }
    }
  }
}
