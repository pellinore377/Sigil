package org.sigil.storage

object NativeStorage {
    init { System.loadLibrary("sigil_android") }
    external fun checkStore(directory: String, key: ByteArray): Boolean
}
