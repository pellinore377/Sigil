package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.graphics.asAndroidBitmap
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class CodeBuilderTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    @Test fun formatting_code_builder_keeps_draft_and_previews_literal_code_before_send() {
        val commands=mutableListOf<Pair<String,Map<String,Any?>>>()
        val chat=ChatSummary("self","@sam:example.test","","",true,emptyList())
        ui.runOnUiThread {ui.activity.setSigilContent {
            SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self"),{name,fields->commands+=name to fields})
        }}
        val body="\tlet x = \"👩🏽‍💻\";\n```\nredact::literal;\n"
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Format").performClick()
        ui.onNodeWithText("Code block").performScrollTo().performClick()
        ui.onNodeWithText("Plain text").performClick()
        ui.onNodeWithText("Rust").performClick()
        ui.onNode(hasSetTextAction() and hasText("Code",substring=false)).performTextInput(body)
        ui.onNodeWithText("Preview code").assertIsDisplayed()
        ui.onNodeWithContentDescription("Back to formatting").performClick()
        ui.onNodeWithText("Code block").performScrollTo().performClick()
        ui.onNode(hasSetTextAction() and hasText("Code",substring=false)).assertTextContains(body)
        assertTrue(commands.none {it.first=="post"})
        ui.onNodeWithText("Preview code").performClick()
        ui.onNodeWithText(body,substring=false).assertExists()
        ui.onNodeWithContentDescription("Copy code").performClick()
        ui.runOnIdle {assertEquals(body,ui.activity.getSystemService(android.content.ClipboardManager::class.java).primaryClip!!.getItemAt(0).text.toString())}
        if(androidx.test.platform.app.InstrumentationRegistry.getArguments().getString("capture_builder")=="true") {
            java.io.File(ui.activity.cacheDir,"code-builder.png").outputStream().use {ui.onRoot().captureToImage().asAndroidBitmap().compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
        }
        ui.onNodeWithText("Send code").assertIsDisplayed().performClick()
        val sent=commands.single {it.first=="post"}.second
        assertEquals(NativeCore.builderSource("Code\nrust\n$body"),sent["text"])
        assertEquals(false,sent["rich"])
        assertEquals(true,sent["formatted"])
    }
}
