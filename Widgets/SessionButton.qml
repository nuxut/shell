import QtQuick
import QtQuick.Layouts
import "../Config"

Rectangle {
    id: root
    
    property string icon: ""
    property string text: ""

    property var action: null // Function to execute
    
    // Confirmation state
    property bool confirmed: false
    
    // Reset timer
    Timer {
        id: resetTimer
        interval: 3000
        onTriggered: root.confirmed = false
    }

    implicitWidth: 100
    implicitHeight: 100
    radius: 10
    color: confirmed ? "#e78284" : (mouseArea.containsMouse ? Config.theme.inactive : Qt.darker(Config.theme.inactive, 1.3))
    
    Behavior on color { ColorAnimation { duration: 150 } }
    
    ColumnLayout {
        anchors.centerIn: parent
        spacing: 5
        
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: root.confirmed ? "\uf0ab" : root.icon // Check mark or X when confirmed? Or just change color?
            // Let's stick to user request: "First click making their color change"
            // So we change BG color to red/warning, and maybe icon shakes?
            // For now, keep icon, maybe change color.
            color: Config.theme.text
            font.family: Config.font.family
            font.pixelSize: 32
        }
    }
    
    // Exclusivity support
    signal requestConfirmation()
    
    function cancelConfirmation() {
        if (confirmed) {
            confirmed = false;
            resetTimer.stop();
        }
    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        
        onClicked: {
            if (!root.confirmed) {
                // First click: Request exclusivity and enter confirmation mode
                root.requestConfirmation();
                root.confirmed = true;
                resetTimer.restart();
            } else {
                // Second click: Execute
                if (root.action) root.action();
                root.confirmed = false; // Reset
            }
        }
    }
}
