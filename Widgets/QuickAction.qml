import QtQuick
import QtQuick.Layouts
import "../Config"

Rectangle {
    id: root
    
    property string icon: ""
    property string text: ""
    property bool isActive: false
    property var action: null
    
    implicitWidth: 140
    implicitHeight: 60
    radius: 10
    color: isActive ? Config.theme.accent : (mouseArea.containsMouse ? Config.theme.inactive : Qt.darker(Config.theme.inactive, 1.3))
    
    Behavior on color { ColorAnimation { duration: 150 } }
    
    RowLayout {
        anchors.centerIn: parent
        spacing: 10
        
        Text {
            text: root.icon
            color: root.isActive ? Config.bar.color : Config.theme.text
            font.family: Config.font.family
            font.pixelSize: 24
        }
        
        Text {
            text: root.text
            color: root.isActive ? Config.bar.color : Config.theme.text
            font.family: Config.font.family
            font.pixelSize: 14
        }
    }
    
    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: {
            if (root.action) root.action();
        }
    }
}
