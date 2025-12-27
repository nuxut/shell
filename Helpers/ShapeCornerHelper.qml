pragma Singleton

import QtQuick
import QtQuick.Shapes
import Quickshell

/**
* ShapeCornerHelper - Utility singleton for shape corner calculations
* Source: Ported from noctalia-shell
*/
Singleton {
  id: root

  // Get X-axis multiplier for a corner state (State 1 = horizontal invert = -1)
  function getMultX(cornerState) {
    return cornerState === 1 ? -1 : 1;
  }

  // Get Y-axis multiplier for a corner state (State 2 = vertical invert = -1)
  function getMultY(cornerState) {
    return cornerState === 2 ? -1 : 1;
  }

  // Get PathArc direction using XOR logic on multipliers
  function getArcDirection(multX, multY) {
    return ((multX < 0) !== (multY < 0)) ? PathArc.Counterclockwise : PathArc.Clockwise;
  }

  // Get flattened radius if dimensions are too small
  function getFlattenedRadius(dimension, requestedRadius) {
    if (dimension < requestedRadius * 2) {
      return dimension / 2;
    }
    return requestedRadius;
  }

  // Check if flattening is needed
  function shouldFlatten(width, height, radius) {
    return width < radius * 2 || height < radius * 2;
  }
}
