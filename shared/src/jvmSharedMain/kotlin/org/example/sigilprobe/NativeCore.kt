package org.sigil

object NativeCore {
    init { System.loadLibrary("sigil_core") }
    external fun palette(accent: Int, dark: Boolean): String
    external fun redact(input: String): String
    external fun analyze(input: String): String
}
