package org.sigil.storage

object NativeStorage {
    external fun scanLinkQr(width: Int, height: Int, bytes: ByteArray): String?
    // Positive handle, negative retry delay in seconds, or zero on failure.
    external fun openCall(directory: String, key: ByteArray, call: ByteArray, tracks: Int): Long
    external fun closeCall(token: Long)
    external fun callState(token: Long): Int
    external fun callTracks(token: Long, tracks: Int): Boolean
    external fun sendCallFrame(token: Long, kind: Int, timestamp: Long, keyframe: Boolean, bytes: ByteArray): Boolean
    external fun receiveCallFrames(token: Long): ByteArray?
    external fun mapResource(directory: String, key: ByteArray, path: String): ByteArray?
    external fun setWallpaper(directory: String, key: ByteArray, peer: String, bytes: ByteArray): Boolean
    external fun wallpaper(directory: String, key: ByteArray, peer: String): ByteArray?
    init { System.loadLibrary("sigil_android") }
    external fun checkStore(directory: String, key: ByteArray): Boolean
    external fun execute(directory: String, key: ByteArray, request: String): String
    external fun stageFile(directory: String, key: ByteArray, request: String, index: Int, bytes: ByteArray): Boolean
    external fun readFileChunk(directory: String, key: ByteArray, peer: String, author: String, message: String, index: Int): ByteArray?
}
