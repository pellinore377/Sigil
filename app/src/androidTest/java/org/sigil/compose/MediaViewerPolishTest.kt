package org.sigil.compose

import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.Density
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*
import java.io.File

class MediaViewerPolishTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    private fun message() = ChatMessage("synthetic-photo", "synthetic-sender", "", false, "9:41", "read", false, emptyList(), emptyList(), null, true,
        peer = "synthetic-conversation", attachment = AttachmentDetails("Synthetic checkerboard", "image/png", 0, caption = "Timeline-only caption"))
    private fun picture() = Bitmap.createBitmap(320, 240, Bitmap.Config.ARGB_8888).apply {
        for (y in 0 until height) for (x in 0 until width) setPixel(x, y, if ((x / 40 + y / 40) % 2 == 0) 0xff377c87.toInt() else 0xffedca8a.toInt())
    }
    private fun capture(name: String) {
        val bitmap = androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
        File(ui.activity.cacheDir, "media-$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        bitmap.recycle()
    }
    @Test fun largeTextViewerKeepsActionsReachableAndReactionsTargetTheSharedMessageAcrossThemes() {
        val current = mutableStateOf(message())
        val dark = mutableStateOf(false)
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        var saved = 0; var options = 0; var closed = 0
        val bitmap = picture()
        ui.runOnUiThread { ui.activity.setSigilContent {
            val density = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(density.density, 1.7f), LocalMediaSender provides { "Maya Chen" }, LocalMediaCommand provides { name, fields ->
                commands += name to fields
                val emoji = fields["emoji"] as String
                current.value = current.value.copy(myReactions = if (fields["active"] == true) current.value.myReactions + emoji else current.value.myReactions - emoji)
            }) {
                MaterialTheme(colorScheme = if (dark.value) darkColorScheme() else lightColorScheme()) {
                    MediaViewerChrome(current.value, { closed++ }, { saved++ }, { options++ }) { Image(bitmap.asImageBitmap(), "Synthetic preview", Modifier.fillMaxSize()) }
                }
            }
        } }
        for (isDark in listOf(false, true)) {
            ui.runOnIdle { dark.value = isDark }
            ui.onNodeWithText("Maya Chen").assertIsDisplayed()
            ui.onNodeWithContentDescription("Save attachment").assertIsDisplayed().performClick()
            ui.onNodeWithContentDescription("Media options").assertIsDisplayed().performClick()
            ui.onNodeWithText("❤️").assertIsDisplayed().performClick()
            ui.runOnIdle {
                assertEquals("react", commands.last().first)
                assertEquals(mapOf("peer" to "synthetic-conversation", "author" to "synthetic-sender", "message" to "synthetic-photo", "emoji" to "❤️", "active" to !isDark), commands.last().second)
            }
            ui.onNodeWithText("Timeline-only caption").assertDoesNotExist()
            capture(if (isDark) "large-dark" else "large-light")
        }
        ui.onNodeWithContentDescription("Close media").assertIsDisplayed().performClick()
        ui.runOnIdle { assertEquals(2, saved); assertEquals(2, options); assertEquals(1, closed) }
    }
    @Test fun imageFallbackStillRespondsToDoubleTapAndAccessibilityZoomWithoutAToolbar() {
        val bitmap = picture()
        ui.runOnUiThread { ui.activity.setSigilContent {
            MaterialTheme { MediaViewerChrome(message(), {}) { ImageViewer(message(), bitmap, Modifier.fillMaxSize()) } }
        } }
        // The zero-byte synthetic attachment fails before storage access; the supplied preview remains usable.
        ui.waitUntil(5000) { ui.onAllNodesWithText("Full-resolution preview is unavailable for this image.").fetchSemanticsNodes().isNotEmpty() }
        val image = ui.onNodeWithContentDescription("Synthetic checkerboard")
        val initial = image.captureToImage().toPixelMap()
        image.performTouchInput { doubleClick(center) }
        val enlarged = image.captureToImage().toPixelMap()
        assertTrue("Double-tap must change the rendered image", (0 until minOf(initial.width, enlarged.width) step 8).sumOf { x ->
            (0 until minOf(initial.height, enlarged.height) step 8).count { y -> initial[x, y] != enlarged[x, y] }
        } > 20)
        val actions = image.fetchSemanticsNode().config[SemanticsActions.CustomActions]
        ui.runOnIdle { assertTrue(actions.single { it.label == "Reset image" }.action()) }
        val reset = image.captureToImage().toPixelMap()
        assertEquals(initial[initial.width / 3, initial.height / 3], reset[reset.width / 3, reset.height / 3])
        ui.onNodeWithText("Fit").assertDoesNotExist()
        ui.onNodeWithContentDescription("Zoom in").assertDoesNotExist()
    }
    @Test fun portraitAndLandscapeMediaStayBetweenChromeInAShortViewport() {
        val dimensions = mutableStateOf(400 to 1200)
        val bitmap = picture()
        ui.runOnUiThread { ui.activity.setSigilContent {
            MaterialTheme {
                CompositionLocalProvider(LocalMediaCommand provides { _, _ -> }) {
                    Box(Modifier.size(320.dp, 380.dp)) {
                        MediaViewerChrome(message(), {}) {
                            MediaViewerFrame(dimensions.value.first, dimensions.value.second) { frame -> Image(bitmap.asImageBitmap(), "Fitted preview", frame.testTag("fitted-media")) }
                        }
                    }
                }
            }
        } }
        for (aspect in listOf(400 to 1200, 1600 to 400)) {
            ui.runOnIdle { dimensions.value = aspect }
            val media = ui.onNodeWithTag("fitted-media").assertIsDisplayed().getUnclippedBoundsInRoot()
            val close = ui.onNodeWithContentDescription("Close media").getUnclippedBoundsInRoot()
            val reactions = ui.onNodeWithText("❤️").getUnclippedBoundsInRoot()
            assertTrue("Media overlaps header", media.top >= close.bottom)
            assertTrue("Media overlaps reactions", media.bottom <= reactions.top)
            assertEquals(aspect.first.toFloat() / aspect.second, (media.right - media.left).value / (media.bottom - media.top).value, .03f)
        }
    }

}
