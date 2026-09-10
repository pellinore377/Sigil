package org.sigil.compose

internal object NativePreview {
    init { System.loadLibrary("sigil_android") }
    external fun render(input: Int, output: Int, format: String, request: String): Boolean
}
