package org.sigil

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import kotlin.test.*

class SecondaryInkTest {
    @Test fun supporting_text_has_hierarchy_without_losing_contrast() {
        for ((ink, surfaces) in listOf(
            Color.White to listOf(Color(0xff151515), Color(0xff202020), Color(0xff444444)),
            Color.Black to listOf(Color(0xfffafafa), Color(0xffededed), Color(0xffcccccc)),
            Color.White to listOf(Color(0xff151515), Color(0xff006677)),
            Color.Black to listOf(Color(0xfffafafa), Color(0xff999900))
        )) {
            val secondary = secondaryInk(ink, surfaces.last(), surfaces)
            assertNotEquals(ink, secondary)
            for (surface in surfaces) {
                val a = secondary.luminance(); val b = surface.luminance()
                assertTrue((maxOf(a,b)+.05f)/(minOf(a,b)+.05f) >= 4.5f)
            }
        }
    }
}
