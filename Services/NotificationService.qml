pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Services.Notifications

Singleton {
    id: root
    
    // Paths
    property string cacheDir: (Quickshell.env("XDG_CACHE_HOME") || Quickshell.env("HOME") + "/.cache") + "/nuxut/"
    property string historyFile: cacheDir + "notifications.json"

    // Component.onCompleted to load history

    Component.onCompleted: {
        console.log("NotificationService initialized");
        Quickshell.execDetached(["mkdir", "-p", cacheDir]);
    }
    property int maxVisible: 5
    property int defaultTimeout: 5000

    // Models
    property ListModel activeList: ListModel {}
    property ListModel historyList: ListModel {}
    
    // Internal state to keep track of notification objects
    property var activeNotifications: ({})

    // Notification server
    NotificationServer {
        id: server
        bodyMarkupSupported: false 
        onNotification: notification => handleNotification(notification)
    }

    // Ringing state for new notifications
    property bool ringing: false
    Timer {
        id: ringTimer
        interval: 3000 // Ring for 3 seconds
        onTriggered: root.ringing = false
    }

    function handleNotification(notification) {
        console.log("Notification received:", notification.summary);
        var id = notification.id.toString();
        
        // Trigger ring
        ringing = true;
        ringTimer.restart();
        
        var data = {
            "id": id,
            "summary": notification.summary || "",
            "body": notification.body || "",
            "appName": notification.appName || "System",
            "appIcon": notification.appIcon || "",
            "urgency": notification.urgency,
            "timeout": notification.expireTimeout > 0 ? notification.expireTimeout : defaultTimeout
        };

        activeNotifications[id] = notification;
        notification.tracked = true;

        notification.closed.connect(function() {
            removeNotification(id);
        });

        activeList.insert(0, data);
        
        // Add to history as well
        historyList.insert(0, data);
        // Limit history size
        if (historyList.count > 50) {
            historyList.remove(50, historyList.count - 50);
        }
        
        saveHistory();

        if (activeList.count > maxVisible) {
             var last = activeList.get(activeList.count - 1);
             dismissNotification(last.id);
        }

        if (data.timeout > 0) {
            // Create a dynamic timer string that calls our specific timeout method
            // We pass the ID as a string literal to the function
            var timerCode = 'import QtQuick; Timer { interval: ' + data.timeout + '; repeat: false; running: true; onTriggered: { parent.timeoutNotification("' + id + '"); destroy(); } }';
            Qt.createQmlObject(timerCode, root, "dynamicTimer");
        }
    }

    // New function to handle timeouts specifically
    function timeoutNotification(id) {
        if (activeNotifications[id]) {
            removeNotification(id);
        }
    }

    function removeNotification(id) {
        var strId = id.toString();
        
        for (var i = 0; i < activeList.count; ++i) {
            if (activeList.get(i).id === strId) {
                activeList.remove(i);
                break;
            }
        }
        
        if (activeNotifications[strId]) {
            delete activeNotifications[strId];
        }
    }

    function dismissNotification(id) {
        var strId = id.toString();
        var notif = activeNotifications[strId];
        if (notif) {
            notif.dismiss();
            removeNotification(strId);
        }
    }

    function clearHistory() {
        historyList.clear();
        saveHistory();
    }
    
    function removeHistoryItem(index) {
        if (index >= 0 && index < historyList.count) {
            historyList.remove(index);
            saveHistory();
        }
    }

    // Persistence
    FileView {
        id: historyFileView
        path: historyFile
        
        onLoaded: {
            if (adapter.items) {
                try {
                    var loadedItems = adapter.items;
                    historyList.clear();
                    for (var i = 0; i < loadedItems.length; i++) historyList.append(loadedItems[i]);
                } catch(e) {
                    console.error("[NotificationService] History error:", e);
                }
            }
        }

        JsonAdapter {
            id: adapter
            property var items: []
        }
    }

    function saveHistory() {
        var items = [];
        for (var i = 0; i < historyList.count; i++) {
            var item = historyList.get(i);
            items.push({
                "id": item.id,
                "summary": item.summary,
                "body": item.body,
                "appName": item.appName,
                "appIcon": item.appIcon,
                "urgency": item.urgency,
                "timeout": item.timeout
            });
        }
        adapter.items = items;
        historyFileView.writeAdapter();
    }
}
