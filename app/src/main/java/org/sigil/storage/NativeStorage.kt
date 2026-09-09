package org.sigil.storage

object NativeStorage {
    init { System.loadLibrary("sigil_android") }
    external fun checkStore(directory: String, key: ByteArray): Boolean
    external fun execute(directory: String, key: ByteArray, request: String): String
}
