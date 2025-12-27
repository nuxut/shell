pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Services.Pipewire

Singleton {
    id: root

    readonly property var defaultSink: Pipewire.defaultAudioSink
    
    PwObjectTracker {
        objects: [defaultSink]
    }
    
    // Volume: 0.0 - 1.0 (or higher if overdrive)
    property real volume: {
        if (defaultSink && defaultSink.audio) {
            return defaultSink.audio.volume
        }
        return 0
    }
    
    property bool muted: {
         if (defaultSink && defaultSink.audio) {
            return defaultSink.audio.muted
        }
        return true
    }

    function setVolume(vol) {
        if (defaultSink && defaultSink.audio) {
            defaultSink.audio.volume = Math.max(0, Math.min(1.5, vol))
        }
    }
    
    function setMuted(mute) {
        if (defaultSink && defaultSink.audio) {
            defaultSink.audio.muted = mute
        }
    }
    
    function increaseVolume() {
        setVolume(volume + 0.05)
    }
    
    function decreaseVolume() {
        setVolume(volume - 0.05)
    }
    
    function toggleMute() {
        setMuted(!muted)
    }

    // Dynamic icon based on volume level
    readonly property string icon: {
        if (muted || volume <= 0) return "\ueee8" // Muted / 0
        if (volume < 0.3) return "\uf026" // < 30%
        if (volume < 0.7) return "\uf027" // 30-70%
        return "\uf028" // > 70%
    }
}
