pragma Singleton
import QtQuick
import Quickshell
import Quickshell.Io

Singleton {
  id: root

  // Public values
  property real cpuUsage: 0
  property real cpuTemp: 0
  property real memPercent: 0
  property real memUsedGb: 0

  // Internal state for CPU calculation
  property var prevCpuStats: null

  // Timers
  Timer {
    interval: 2000
    repeat: true
    running: true
    triggeredOnStart: true
    onTriggered: {
      cpuStatFile.reload();
      memInfoFile.reload();
      updateCpuTemperature();
    }
  }

  // Files
  FileView {
    id: cpuStatFile
    path: "/proc/stat"
    onLoaded: calculateCpuUsage(text())
  }

  FileView {
    id: memInfoFile
    path: "/proc/meminfo"
    onLoaded: parseMemoryInfo(text())
  }

  FileView {
    id: cpuTempReader
    printErrors: false
    onLoaded: {
      root.cpuTemp = Math.round(parseInt(text().trim()) / 1000.0);
    }
  }

  // Detection logic for CPU temp
  property string tempPath: ""

  Process {
    id: sensorFinder
    command: ["sh", "-c", "grep -lE 'coretemp|k10temp|zenpower' /sys/class/hwmon/hwmon*/name | head -n1"]
    running: true
    stdout: SplitParser {
      onRead: (data) => {
        // data e.g. "/sys/class/hwmon/hwmon5/name"
        if (data && data.trim().length > 0) {
           let basePath = data.trim();
           // Replace '/name' with '/temp1_input'
           root.tempPath = basePath.replace("/name", "/temp1_input");
           cpuTempReader.path = root.tempPath;
           cpuTempReader.reload();
        }
      }
    }
  }

  Component.onCompleted: {
    // Process runs automatically due to running: true
    // but just in case we can restart it if needed
  }

  function updateCpuTemperature() {
    if (tempPath) {
      cpuTempReader.path = tempPath;
      cpuTempReader.reload();
    }
  }

  function calculateCpuUsage(text) {
    if (!text) return;
    const lines = text.split('\n');
    const cpuLine = lines[0];
    if (!cpuLine.startsWith('cpu ')) return;
    
    const parts = cpuLine.split(/\s+/).filter(p => p.length > 0);
    const stats = {
      user: parseInt(parts[1]),
      nice: parseInt(parts[2]),
      system: parseInt(parts[3]),
      idle: parseInt(parts[4]),
      iowait: parseInt(parts[5]),
      irq: parseInt(parts[6]),
      softirq: parseInt(parts[7]),
      steal: parseInt(parts[8])
    };
    
    const idle = stats.idle + stats.iowait;
    const total = stats.user + stats.nice + stats.system + stats.idle + stats.iowait + stats.irq + stats.softirq + stats.steal;

    if (prevCpuStats) {
      const diffIdle = idle - prevCpuStats.idle;
      const diffTotal = total - prevCpuStats.total;
      if (diffTotal > 0) {
        root.cpuUsage = Math.round(((diffTotal - diffIdle) / diffTotal) * 100);
      }
    }

    prevCpuStats = { idle: idle, total: total };
  }

  function parseMemoryInfo(text) {
    if (!text) return;
    const lines = text.split('\n');
    let memTotal = 0;
    let memAvailable = 0;

    for (const line of lines) {
      if (line.startsWith('MemTotal:')) {
        memTotal = parseInt(line.split(/\s+/)[1]);
      } else if (line.startsWith('MemAvailable:')) {
        memAvailable = parseInt(line.split(/\s+/)[1]);
      }
    }

    if (memTotal > 0) {
      let used = memTotal - memAvailable;
      root.memUsedGb = (used / (1024 * 1024)).toFixed(1);
      root.memPercent = Math.round((used / memTotal) * 100);
    }
  }
}
