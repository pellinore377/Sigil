package org.sigil

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class ImageMessageFrameTest {
    @Test fun portraitLandscapeAndSquareKeepTheirAspectWithoutLetterboxing() {
        listOf(600 to 1200, 1600 to 900, 800 to 800, 40 to 2000, 2000 to 40).forEach { (w,h) ->
            val size=imageMessageSize(w,h,280f)
            assertEquals(w.toFloat()/h,size.width/size.height,.0001f)
            assertTrue(size.width<=280f && size.height<=360f)
        }
    }
    @Test fun missingDimensionsHaveFiniteLoadingSpace() {
        assertEquals(androidx.compose.ui.geometry.Size(300f,300f),imageMessageSize(0,0,400f))
    }
}
