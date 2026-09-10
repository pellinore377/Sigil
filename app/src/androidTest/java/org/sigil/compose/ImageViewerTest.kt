package org.sigil.compose

import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Rect
import android.media.ExifInterface
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Test
import org.junit.Assert.*

class ImageViewerTest {
    @Test fun regionsFollowAllExifOrientationsWithoutChangingPixelContent() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val colors = intArrayOf(Color.RED, Color.GREEN, Color.BLUE, Color.YELLOW)
        val corners = arrayOf(intArrayOf(0, 1, 2, 3), intArrayOf(1, 0, 3, 2), intArrayOf(3, 2, 1, 0), intArrayOf(2, 3, 0, 1),
            intArrayOf(0, 2, 1, 3), intArrayOf(2, 0, 3, 1), intArrayOf(3, 1, 2, 0), intArrayOf(1, 3, 0, 2))
        val original = Bitmap.createBitmap(80, 40, Bitmap.Config.ARGB_8888)
        for (y in 0 until 40) for (x in 0 until 80) original.setPixel(x, y, colors[(if (x >= 40) 1 else 0) + (if (y >= 20) 2 else 0)])
        val file = java.io.File(context.cacheDir, "synthetic-orientation.jpg")
        try {
            for (orientation in 1..8) {
                file.outputStream().use { original.compress(Bitmap.CompressFormat.JPEG, 100, it) }
                ExifInterface(file.path).apply { setAttribute(ExifInterface.TAG_ORIENTATION, orientation.toString()); saveAttributes() }
                val bytes = file.readBytes(); file.delete()
                val source = RegionSource(bytes)
                try {
                    assertEquals(if (orientation >= 5) 40 else 80, source.width)
                    assertEquals(if (orientation >= 5) 80 else 40, source.height)
                    val full = source.region(Rect(0, 0, source.width, source.height), 1).bitmap
                    try {
                        for (corner in 0..3) {
                            val rect = Rect((corner % 2) * source.width / 2, (corner / 2) * source.height / 2, (corner % 2 + 1) * source.width / 2, (corner / 2 + 1) * source.height / 2)
                            val tile = source.region(rect, 1)
                            try {
                                assertEquals(rect, tile.rect)
                                val expected = colors[corners[orientation - 1][corner]]
                                val actual = tile.bitmap.getPixel(tile.bitmap.width / 2, tile.bitmap.height / 2)
                                assertEquals(full.getPixel(rect.centerX(), rect.centerY()), actual)
                                assertTrue("orientation=$orientation corner=$corner", kotlin.math.abs(Color.red(expected) - Color.red(actual)) < 8 && kotlin.math.abs(Color.green(expected) - Color.green(actual)) < 8 && kotlin.math.abs(Color.blue(expected) - Color.blue(actual)) < 8)
                            } finally { tile.bitmap.recycle() }
                        }
                    } finally { full.recycle() }
                } finally { source.close() }
                assertTrue(bytes.all { it == 0.toByte() })
                assertThrows(IllegalStateException::class.java) { source.region(Rect(0, 0, 1, 1), 1) }
            }
        } finally { file.delete(); original.recycle() }
    }
}
