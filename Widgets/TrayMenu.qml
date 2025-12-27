import QtQuick
import QtQuick.Layouts
import QtQuick.Controls
import Quickshell
import Quickshell.Services.SystemTray
import "./../Config"

PopupWindow {
    id: root
    
    // API
    property var menuHandle: null
    property var anchorItem: null
    property bool isSubMenu: false
    property bool externalHover: false
    property int hoverCount: 0
    readonly property bool isHovered: hoverCount > 0
    
    // Auto Close Logic
    Timer {
        id: closeTimer
        interval: 500
        repeat: false
        onTriggered: {
            if (!root.isHovered && !root.externalHover && !root.isSubMenuOpen) {
                root.close();
            }
        }
    }

    function checkAutoClose() {
        if (!root.isHovered && !root.externalHover && !root.isSubMenuOpen) {
            closeTimer.restart();
        } else {
            closeTimer.stop();
        }
    }

    onExternalHoverChanged: checkAutoClose()
    onIsHoveredChanged: checkAutoClose()


    


    implicitWidth: 200
    implicitHeight: layout.implicitHeight + 10
    color: "transparent"
    visible: false
    
    onVisibleChanged: {
        if (visible) {
            focusItem.forceActiveFocus();
        }
    }
    
    Item {
        id: focusItem
        focus: true
        anchors.fill: parent
        Keys.onEscapePressed: root.close()
        
        onActiveFocusChanged: {
            if (!activeFocus && root.visible) {
                // We lost focus, likely clicked outside
                // Delay slightly to allow click processing? No, immediate is fine usually.
                root.close();
            }
        }
    }

    // Anchoring logic
    property var parentWindow: null
    anchor.window: parentWindow
    anchor.item: anchorItem
    
    // Position: If submenu, to the right; if root, below/above based on bar
    anchor.rect.x: isSubMenu ? parent.width : (anchorItem ? -root.width + anchorItem.width : 0)
    anchor.rect.y: isSubMenu ? 0 : (anchorItem ? anchorItem.height + 5 : 0)

    // Data Source
    QsMenuOpener {
        id: opener
        menu: root.menuHandle
    }

    // Background
    Rectangle {
        anchors.fill: parent
        color: Config.bar.color
        radius: 5
        border.color: "#333"
        border.width: 1
        
        // Prevent click-through and track hover
        MouseArea { 
            id: backgroundMouse
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons 
            hoverEnabled: true 
            
            onEntered: {
                root.hoverCount++;
            }
            onExited: {
                root.hoverCount--;
            }
        }


        ColumnLayout {
            id: layout
            width: parent.width
            spacing: 0
            
            
            Repeater {
                model: (opener.children && opener.children.values) ? [...opener.children.values] : []
                
                delegate: Rectangle {
                    id: menuItem
                    Layout.fillWidth: true
                    Layout.preferredHeight: modelData.isSeparator ? 2 : 28
                    color: itemMouse.containsMouse ? "#33ffffff" : "transparent"
                    visible: modelData.visible !== undefined ? modelData.visible : true
                    radius: 3

                    property var subMenu: null

                    // Separator visual
                    Rectangle {
                        anchors.centerIn: parent
                        width: parent.width - 10
                        height: 1
                        color: "#666"
                        visible: modelData.isSeparator
                    }

                    // Content
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 8
                        visible: !modelData.isSeparator
                        spacing: 8

                        // Text
                        Text {
                            Layout.fillWidth: true
                            text: modelData.text.replace(/&/g, "")
                            color: Config.theme.text
                            font.family: Config.font.family
                            font.pixelSize: 13
                            elide: Text.ElideRight
                            verticalAlignment: Text.AlignVCenter
                        }

                        // Submenu Indicator
                        Text {
                            text: "›"
                            color: "#aaa"
                            font.pixelSize: 18
                            visible: modelData.hasChildren
                        }
                    }

                    MouseArea {
                        id: itemMouse
                        anchors.fill: parent
                        hoverEnabled: !modelData.isSeparator
                        enabled: !modelData.isSeparator
                        acceptedButtons: Qt.LeftButton
                        
                        onEntered: root.hoverCount++
                        onExited: root.hoverCount--
                        
                        onClicked: {
                            if (modelData.hasChildren) {
                                // Submenu Logic
                                if (menuItem.subMenu) {
                                    menuItem.subMenu.visible = !menuItem.subMenu.visible;
                                    if (!menuItem.subMenu.visible) {
                                        menuItem.subMenu.destroy();
                                        menuItem.subMenu = null;
                                    }
                                } else {
                                    // Close siblings
                                    closeSiblings();
                                    
                                    // Open new
                                    var comp = Qt.createComponent("TrayMenu.qml");
                                    if (comp.status === Component.Ready) {
                                        menuItem.subMenu = comp.createObject(root, {
                                            menuHandle: modelData,
                                            anchorItem: menuItem,
                                            parentWindow: root,
                                            isSubMenu: true
                                        });
                                        // The component manages its own visibility based on opener
                                        menuItem.subMenu.open(modelData, menuItem);
                                        root.isSubMenuOpen = true; // Mark submenu as open which prevents auto-close
                                        
                                        // When submenu closes, we should know
                                        menuItem.subMenu.onVisibleChanged.connect(function() {
                                            if (menuItem.subMenu && !menuItem.subMenu.visible) {
                                                root.isSubMenuOpen = false;
                                                root.checkAutoClose();
                                            }
                                        });

                                    } else {
                                        console.log("Error loading submenu:", comp.errorString());
                                    }
                                }
                            } else {
                                // Action Logic
                                modelData.triggered();
                                root.closeAll();
                            }
                        }
                    }
                    
                    function closeSiblings() {}
                }
            }
        }
    }

    function open(menu, anchor) {
        root.menuHandle = menu;
        root.anchorItem = anchor;
        root.visible = true;
    }
    
    function close() {
        if (root.visible) {
            root.visible = false;
        }
    }
    
    function closeAll() {
        root.close();
        // Propagate if needed
    }
}
