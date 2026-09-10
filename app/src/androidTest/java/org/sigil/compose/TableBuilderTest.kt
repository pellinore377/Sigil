package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.graphics.asAndroidBitmap
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class TableBuilderTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    @Test fun table_builder_preserves_fields_on_back_and_only_sends_from_preview() {
        val commands=mutableListOf<Pair<String,Map<String,Any?>>>()
        val chat=ChatSummary("self","@sam:example.test","","",true,emptyList())
        ui.runOnUiThread {ui.activity.setSigilContent {
            SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self"),{name,fields->commands+=name to fields})
        }}
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Create").performClick()
        ui.onNodeWithContentDescription("Table").performScrollTo().performClick()
        ui.onNodeWithText("Column 1").performTextInput("Name")
        ui.onNodeWithText("Column 2").performScrollTo().performTextInput("Count")
        ui.onNode(hasScrollToIndexAction() and !hasTestTag("timeline")).performScrollToNode(hasText("Enter rows"))
        ui.onNodeWithText("Enter rows").performClick()
        ui.onNodeWithText("Name").performTextInput("Fish |\nchips")
        ui.onNodeWithText("Count").performScrollTo().performTextInput("2")
        ui.onNodeWithContentDescription("Back to create").performClick()
        ui.onNodeWithContentDescription("Table").performScrollTo().performClick()
        ui.onNodeWithText("Rows").assertIsSelected()
        ui.onNodeWithText("Name").assertTextContains("Fish | chips")
        assertTrue(commands.none {it.first=="post"})
        ui.onNode(hasScrollToIndexAction() and !hasTestTag("timeline")).performScrollToNode(hasText("Preview table"))
        ui.onNodeWithText("Preview table").performClick()
        if(androidx.test.platform.app.InstrumentationRegistry.getArguments().getString("capture_builder")=="true") {
            java.io.File(ui.activity.cacheDir,"table-builder.png").outputStream().use {ui.onRoot().captureToImage().asAndroidBitmap().compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
        }
        ui.onNodeWithText("Send table").performScrollTo().performClick()
        val sent=commands.single {it.first=="post"}.second
        assertEquals(NativeCore.builderSource("Table\nName\tCount\nFish | chips\t2"),sent["text"])
        assertEquals(true,sent["rich"])
    }
}
