import QtQuick
import QtQuick.Layouts
import QtQuick.Controls
import Quickshell
import Quickshell.Wayland
import "../Config"

PanelWindow {
    id: root
    
    property bool isOpen: false
    property bool externalHover: false
    property bool internalHover: false
    property var lockScreen: null
    
    // Auto-close logic
    Timer {
        interval: 500
        repeat: true
        running: isOpen
        onTriggered: {
            if (!internalHover && !externalHover) {
                isOpen = false;
            }
        }
    }

    anchors {
        top: true
        left: true
    }
    
    // Position it below the visual bar (30px)
    margins.left: 0
    // margins.top will be animated

    implicitWidth: 400
    implicitHeight: 600
    
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.namespace: "nuxut-left-panel"
    WlrLayershell.exclusionMode: ExclusionMode.Ignore
    
    // Animation Logic
    property real showProgress: isOpen ? 100 : 0 // 0 to 100
    Behavior on showProgress { NumberAnimation { duration: 300; easing.type: Easing.OutCubic } }
    
    // Visible as long as it's partially shown
    visible: showProgress > 0
    
    // Animate position: 29 when open, -height when closed
    margins.top: 29 - (1.0 - (showProgress / 100.0)) * (root.implicitHeight + 50)
    
    color: "transparent"
    
    Rectangle {
        anchors.fill: parent
        color: Config.bar.color
        radius: Config.bar.radius
        
        HoverHandler {
            id: hoverHandler
            onHoveredChanged: root.internalHover = hovered
        }

        // Square top corners to merge with bar
        Rectangle {
            height: parent.radius
            width: parent.width
            color: parent.color
            anchors.top: parent.top
        }
        
        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 20
            spacing: 20
            
            // Header
            Text {
                text: "Session Control"
                color: Config.theme.text
                font.family: Config.font.family
                font.pixelSize: 12
                font.bold: true
                Layout.fillWidth: true
            }
            
            // Session Controls
            RowLayout {
                spacing: 10
                Layout.fillWidth: true
                Layout.preferredHeight: 60
                
                // Track currently active (confirming) button
                property var activeSessionBtn: null
                
                property var sessionButtons: [
                    { 
                        icon: "\uf011", 
                        action: () => Quickshell.execDetached(["systemctl", "poweroff"]) 
                    },
                    { 
                        icon: "\uf021", 
                        action: () => Quickshell.execDetached(["systemctl", "reboot"]) 
                    },
                    { 
                        icon: "\uf023", 
                        action: () => Quickshell.execDetached("hyprlock", []) 
                    },
                    { 
                        icon: "\uf2f5", 
                        action: () => Quickshell.execDetached(["hyprctl", "dispatch", "exit"]) 
                    }
                ]
                
                Repeater {
                    model: parent.sessionButtons
                    
                    SessionButton {
                        Layout.fillWidth: true
                        Layout.preferredHeight: 60
                        icon: modelData.icon
                        action: modelData.action
                        
                        onRequestConfirmation: {
                            // Reset any other button that is currently in confirmation mode
                            if (parent.activeSessionBtn && parent.activeSessionBtn !== this) {
                                parent.activeSessionBtn.cancelConfirmation()
                            }
                            parent.activeSessionBtn = this
                        }
                    }
                }
            }
            
            Item { height: 10 } // Spacer
            
            Text {
                text: "Utilities"
                color: Config.theme.text
                font.family: Config.font.family
                font.pixelSize: 12
                font.bold: true
                Layout.fillWidth: true
            }
            
            // Utilities
            ColumnLayout {
                spacing: 10
                Layout.fillWidth: true
                
                QuickAction {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 60
                    icon: "\uf002" // Search
                    text: "App Launcher"
                    action: () => {
                        Quickshell.execDetached(["fuzzel"]);
                        root.isOpen = false;
                    }
                }
                
                property bool caffeine: false
                property var caffeineProc: null
                
                QuickAction {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 60
                    icon: "\uf0f4" // Coffee
                    text: parent.caffeine ? "Caffeine On" : "Caffeine Off"
                    isActive: parent.caffeine
                    action: () => {
                        parent.caffeine = !parent.caffeine;
                        if (parent.caffeine) {
                             // Use systemd-inhibit to block idle
                             parent.caffeineProc = Quickshell.execDetached(["systemd-inhibit", "--what=idle", "--who=nuxut", "--why=caffeine", "sleep", "infinity"]);
                        } else {
                             // Kill the inhibition process
                             Quickshell.execDetached(["pkill", "-f", "nuxut.*caffeine"]);
                        }
                    }
                }
            }
            
            Item { Layout.fillHeight: true } // Push to top
        }
    }
}
