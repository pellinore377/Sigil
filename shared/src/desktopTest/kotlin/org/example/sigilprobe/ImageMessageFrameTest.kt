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
    @Test fun missingDimensionsHoldATypicalPictureHeightRatherThanAFullWidthSquare() {
        val pending=imageMessageSize(0,0,400f)
        assertEquals(MessageBubbleMaxWidth,pending.width)
        assertEquals(PendingPictureRatio,pending.width/pending.height,.0001f)
        assertTrue(pending.height<MessageBubbleMaxWidth,"a pending picture is shorter than a full-width square")
    }
}
