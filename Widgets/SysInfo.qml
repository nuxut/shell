import QtQuick
import QtQuick.Layouts
import Quickshell
import "../Config"
import "../Services"

Item {
  id: root
  implicitWidth: layout.implicitWidth
  implicitHeight: parent ? parent.height : 30

  RowLayout {
    id: layout
    anchors.centerIn: parent
    spacing: 8
  
    readonly property color textColor: Config.theme.text

    // CPU
    Row {
      spacing: 2
      Text {
        text: "\uf4bc" // nf-fa-microchip
        color: "#e78284" // Frappe Red
        font.family: Config.font.family
        font.pixelSize: Config.font.size
      }
      Text {
        text: SystemStatService.cpuUsage + "%"
        color: layout.textColor
        font.family: Config.font.family
        font.pixelSize: Config.font.size - 2
      }
    }

    // Temp
    Row {
      spacing: 2
      Text {
        text: "\uf2c9" // nf-fa-thermometer_half
        color: "#ef9f76" // Frappe Peach
        font.family: Config.font.family
        font.pixelSize: Config.font.size
      }
      Text {
        text: SystemStatService.cpuTemp + "°C"
        color: layout.textColor
        font.family: Config.font.family
        font.pixelSize: Config.font.size - 2
      }
    }

    // RAM
    Row {
      spacing: 2
      Text {
        text: "\uefc5" // nf-md-memory
        color: "#a6d189" // Frappe Green
        font.family: Config.font.family
        font.pixelSize: Config.font.size
      }
      Text {
        text: SystemStatService.memUsedGb + " GB"
        color: layout.textColor
        font.family: Config.font.family
        font.pixelSize: Config.font.size - 2
      }
    }
  }
}
