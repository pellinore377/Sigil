import QtQuick
import "../video"

// The only file that imports the native VideoSurface plugin (isolated so a
// missing/incompatible .so degrades to avatar tiles instead of breaking Panel.qml).
VideoSurface {
  id: root
  property string shmPath: ""
  property bool mirrored: false
  source: shmPath
  fillMode: VideoSurface.PreserveAspectCrop
  mirror: mirrored
}
