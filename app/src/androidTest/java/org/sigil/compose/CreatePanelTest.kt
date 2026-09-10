package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.core.view.WindowInsetsCompat
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class CreatePanelTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    @Test fun tool_search_stays_above_the_keyboard_and_returns_to_its_results() {
        val commands=mutableListOf<String>()
        val chat=ChatSummary("self","@sam:example.test","","",true,emptyList())
        ui.runOnUiThread {ui.activity.setSigilContent {
            SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self"),{name,_->commands+=name})
        }}
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Create").performClick()
        ui.onNodeWithContentDescription("Plan & Organize").assertIsDisplayed()
        if(androidx.test.platform.app.InstrumentationRegistry.getArguments().getString("capture_builder")=="true") {
            java.io.File(ui.activity.cacheDir,"create-panel.png").outputStream().use {ui.onRoot().captureToImage().asAndroidBitmap().compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
        }
        ui.onNodeWithText("Search tools").performClick().performTextInput("coin")
        fun keyboardBottom()=WindowInsetsCompat.toWindowInsetsCompat(ui.activity.window.decorView.rootWindowInsets).getInsets(WindowInsetsCompat.Type.ime()).bottom
        ui.waitUntil(5000) {keyboardBottom()>0}
        val tool=ui.onNodeWithContentDescription("Randomizer")
        tool.assertIsDisplayed()
        assertTrue(tool.fetchSemanticsNode().boundsInWindow.bottom<=ui.activity.window.decorView.height-keyboardBottom()+2)
        tool.performClick()
        ui.onNodeWithText("Count 1").assertExists()
        ui.onNodeWithContentDescription("Back to create").performClick()
        ui.onNodeWithText("Search tools").assertTextContains("coin")
        ui.onNodeWithContentDescription("Randomizer").assertExists()
        ui.onNodeWithContentDescription("Clear tool search").performClick()
        ui.onNodeWithContentDescription("Ask & Decide").performClick()
        ui.onNodeWithContentDescription("Poll").assertExists()
        ui.onNodeWithContentDescription("Back to categories").performClick()
        ui.onNodeWithContentDescription("Back to attachments").performClick()
        ui.onNodeWithContentDescription("Photos").assertExists()
        assertFalse(commands.any {it in listOf("post","contact_request")})
    }
}
