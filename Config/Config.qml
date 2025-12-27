pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io

Singleton {
  id: root

  // Config Paths
  readonly property string configDir: (Quickshell.env("XDG_CONFIG_HOME") || Quickshell.env("HOME") + "/.config") + "/nuxut/quickshell/"
  readonly property string configFile: configDir + "config.json"

  // Initialization
  Component.onCompleted: {
    // Ensure config directory exists
    Quickshell.execDetached(["mkdir", "-p", configDir]);
  }

  property bool initialLoadDone: false

  FileView {
    id: fileView
    path: configFile
    watchChanges: true
    
    // Create file if it doesn't exist
    onLoadFailed: function(error) {
      if (error.toString().includes("No such file") || error === 2) fileView.writeAdapter();
    }
    
    onFileChanged: {
      console.log("Config changed, reloading...");
      reload();
    }
  }

  // Configuration Data
  property alias bar: adapter.bar
  property alias font: adapter.font
  property alias theme: adapter.theme

  JsonAdapter {
    id: adapter
    
    // Bar settings
    property JsonObject bar: JsonObject {
      property string color: "#303446" // Catppuccin Frappe Base
      property real radius: 20
      property real bottomGap: 2
    }

    property JsonObject font: JsonObject {
      property string family: "Ubuntu Nerd Font Propo Med"
      property int size: 12
      property bool bold: true
    }

    // Theme settings
    property JsonObject theme: JsonObject {
      property string accent: "#85c1dc" // Catppuccin Frappe Sapphire
      property string text: "#c6d0f5"   // Catppuccin Frappe Text
      property string inactive: "#51576d" // Catppuccin Frappe Surface1
    }
  }
  
  // Bind the adapter to the file view
  Binding {
    target: fileView
    property: "adapter"
    value: adapter
  }

  // Auto-merge logic:
  // When file is loaded for the first time, we write back. 
  // This ensures that new default keys (like 'font') are injected into the file.
  Connections {
    target: fileView
    function onLoaded() {
       if (!root.initialLoadDone) {
         root.initialLoadDone = true;
       }
    }
  }
}
