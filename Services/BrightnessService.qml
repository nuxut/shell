pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io

Singleton {
    id: root

    property real brightness: 0.5 // Default fallback
    property real maxBrightness: 100
    property string deviceName: ""
    
    // Polling process to get brightness
    // brightnessctl -m returns: generic,backlight,current,max,percentage
    Process {
        id: infoProc
        command: ["brightnessctl", "-m"]
        running: true
        stdout: StdioCollector {
            onStreamFinished: {
                if (root.ignoringUpdates) {
                     timer.restart();
                     return;
                }
                
                var lines = text.trim().split("\n");
                for (var i = 0; i < lines.length; i++) {
                    var parts = lines[i].split(",");
                    if (parts.length >= 4 && parts.length <= 6) { 
                       
                       // Find percent
                       var pct = -1;
                       var max = -1;
                       
                       for (var j=0; j<parts.length; j++) {
                           if (parts[j].indexOf("%") !== -1) {
                               pct = parseInt(parts[j]);
                           }
                       }
                       
                       if (parts[1] === "backlight") {
                           if (parts[3].indexOf("%") !== -1) {
                               pct = parseInt(parts[3]);
                               if (parts.length > 4) max = parseInt(parts[parts.length-1]);
                               else max = 100;
                           } else if (parts.length > 4 && parts[4].indexOf("%") !== -1) {
                               pct = parseInt(parts[4]);
                               max = parseInt(parts[3]);
                           }
                           
                           if (pct !== -1 && !isNaN(pct)) {
                               root.brightness = Math.min(1.0, Math.max(0.01, pct / 100.0));
                               if (max !== -1 && !isNaN(max)) root.maxBrightness = max;
                               break; 
                           }
                       }
                    }
                }
                // Reschedule
                timer.restart();
            }
        }
    }
    
    // Timer to poll
    Timer {
        id: timer
        interval: 2000 // Poll every 2 seconds
        onTriggered: infoProc.running = true
    }
    
    // Process to set brightness
    Process {
        id: setProc
    }

    property bool ignoringUpdates: false
    Timer {
        id: ignoreTimer
        interval: 2000
        onTriggered: root.ignoringUpdates = false
    }

    function setBrightnessProc(val) {
        // Enforce > 1%
        var safeVal = Math.max(0.01, Math.min(1.0, val));
        var percent = Math.round(safeVal * 100) + "%";
        
        root.ignoringUpdates = true;
        ignoreTimer.restart();
        
        setProc.command = ["brightnessctl", "s", percent];
        setProc.running = true;
        
        // Optimistic update
        root.brightness = safeVal;
    }

    function increaseBrightness() {
        setBrightnessProc(brightness + 0.10);
    }

    function decreaseBrightness() {
        setBrightnessProc(brightness - 0.10);
    }

    readonly property string icon: {
        if (brightness >= 0.90) return "\udb80\udce0" // 90% +
        if (brightness >= 0.70) return "\udb80\udcdd" // 70-90%
        if (brightness >= 0.30) return "\udb80\udcdf" // 30-70%
        return "\udb80\udcde" // < 30%
    }
}
