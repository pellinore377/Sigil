package org.sigil

object NativeCore {
    init { System.loadLibrary("sigil_core") }
    external fun palette(accent: Int, dark: Boolean): String
    external fun redact(input: String): String
    external fun analyze(input: String): String
    external fun editor(input: String): String
    external fun motionSeeds(input: String): String
    external fun temporalPreview(input: String): String
    external fun helpCatalog(input: String): String
    external fun builderSource(input: String): String
}
