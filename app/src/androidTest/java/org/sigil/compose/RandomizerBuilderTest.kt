package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.graphics.asAndroidBitmap
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class RandomizerBuilderTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    @Test fun create_keeps_randomizers_in_the_composer_and_sends_canonical_content_explicitly() {
        val commands=mutableListOf<Pair<String,Map<String,Any?>>>()
        val chat=ChatSummary("self","@sam:example.test","","",true,emptyList())
        ui.runOnUiThread {ui.activity.setSigilContent {
            SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self"),{name,fields->commands+=name to fields})
        }}
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Create").performClick()
        ui.onNodeWithContentDescription("Ask & Decide").performClick()
        ui.onNodeWithContentDescription("Randomizer").performScrollTo().performClick()
        ui.onNodeWithText("Count 1").performTextReplacement("3")
        ui.onNodeWithText("Sides 1").performTextReplacement("8")
        assertTrue(commands.none {it.first=="post"})
        ui.onNodeWithText("Roll dice").performScrollTo().performClick()
        val dice=commands.single {it.first=="post"}.second
        assertEquals("roll::3d8;",dice["text"])
        assertEquals(true,dice["rich"])
        ui.onNodeWithText("Choice").performScrollTo().performClick()
        ui.onNodeWithText("Choice 1").performTextInput("Fish,\nchips")
        ui.onNodeWithText("Choice 2").performScrollTo().performTextInput("redact::literal;")
        ui.onNodeWithText("Pick a choice").performScrollTo().performClick()
        assertEquals(NativeCore.builderSource("Choice\nFish, chips\nredact::literal;"),commands.last {it.first=="post"}.second["text"])
        ui.onNodeWithText("Coin").performScrollTo().performClick()
        ui.onNodeWithText("Coin").assertIsSelected()
        ui.onNodeWithText("Flip coin").performScrollTo().performClick()
        assertEquals("pick::flip;",commands.last {it.first=="post"}.second["text"])
        if(androidx.test.platform.app.InstrumentationRegistry.getArguments().getString("capture_builder")=="true") {
            java.io.File(ui.activity.cacheDir,"randomizer-builder.png").outputStream().use {ui.onRoot().captureToImage().asAndroidBitmap().compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
        }
        ui.onNodeWithContentDescription("Back to create").performClick()
        ui.onNodeWithContentDescription("Back to categories").assertIsDisplayed()
        assertEquals(3,commands.count {it.first=="post"})
    }
}
