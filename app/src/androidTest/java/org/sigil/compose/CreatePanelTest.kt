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
    @Test fun pages_preserve_selection_without_search_or_sending() {
        val commands=mutableListOf<String>()
        val chat=ChatSummary("self","@sam:example.test","","",true,emptyList())
        ui.runOnUiThread {ui.activity.setSigilContent {
            SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self"),{name,_->commands+=name})
        }}
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Create").performClick()
        ui.onNodeWithContentDescription("Create page 1").assertIsDisplayed()
        if(androidx.test.platform.app.InstrumentationRegistry.getArguments().getString("capture_builder")=="true") {
            java.io.File(ui.activity.cacheDir,"create-panel.png").outputStream().use {ui.onRoot().captureToImage().asAndroidBitmap().compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
        }
        ui.onNodeWithText("Search tools").assertDoesNotExist()

        ui.onNodeWithContentDescription("Dice").assertIsDisplayed().performClick()
        ui.onNodeWithText("Count 1").assertExists()
        ui.onNodeWithContentDescription("Back to create").performClick()
        ui.onNodeWithContentDescription("Dice").assertIsDisplayed()
        ui.onNodeWithContentDescription("Poll").assertIsDisplayed()
        ui.onNodeWithTag("create-pages").performTouchInput {swipeLeft()}
        ui.onNodeWithContentDescription("Create page 2").assertIsSelected()
        ui.onNodeWithContentDescription("Table").performClick()
        ui.onNodeWithContentDescription("Back to create").performClick()
        ui.onNodeWithContentDescription("Create page 2").assertIsSelected()
        ui.onNodeWithContentDescription("Back to attachments").performClick()
        ui.onNodeWithContentDescription("Photos").assertExists()
        assertFalse(commands.any {it in listOf("post","contact_request")})
    }
}
