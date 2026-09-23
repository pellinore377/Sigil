package org.sigil.compose

import android.view.View
import android.view.ViewGroup
import android.view.inspector.WindowInspector
import android.webkit.WebView
import androidx.activity.ComponentActivity
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class UtilityTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    private var utility by mutableStateOf(UtilityContent("calculation"))
    private fun show(value: UtilityContent) {
        utility = value
        val chat = ChatSummary("self","@sam:example.test","","",true,emptyList())
        ui.runOnUiThread { ui.activity.setSigilContent {
            val message = ChatMessage("utility","sam","Utility",true,"9:33","sent",false,emptyList(),emptyList(),null,true,
                timestamp=1000,parts=listOf(MessagePart("card","utility","Utility",utility=utility)))
            SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self",messages=listOf(message)),{_,_->})
        } }
    }
    @Test fun qr_concealment_requires_reveal_before_scanning_or_copying() {
        val cells = (0 until 29*29).map { if (it / 29 in 4..24 && it % 29 in 4..24 && it % 2==0) '1' else '0' }.joinToString("")
        show(UtilityContent("qr",rich=RichText("Synthetic network"),qr=QrContent("wifi",29,cells,"WIFI:T:WPA;S:Synthetic;P:synthetic-secret;;",RichText("synthetic-secret"),true)))
        ui.onNodeWithContentDescription("Scannable QR code").assertDoesNotExist()
        ui.onNodeWithText("Open wi-fi qr code").performClick()
        ui.onNodeWithText("Copy Wi-Fi details").assertDoesNotExist()
        ui.onNode(hasText("Reveal QR code") and hasAnyAncestor(isDialog())).performClick()
        val bitmap = ui.onNode(hasContentDescription("Scannable QR code") and hasAnyAncestor(isDialog())).captureToImage().asAndroidBitmap()
        for (x in 0 until bitmap.width) assertEquals(android.graphics.Color.WHITE,bitmap.getPixel(x,0))
        ui.onNodeWithText("synthetic-secret").assertDoesNotExist()
        ui.onNodeWithText("Show password").performScrollTo().performClick()
        ui.onNodeWithText("synthetic-secret").assertIsDisplayed()
        ui.onNodeWithText("Copy Wi-Fi details").performScrollTo().performClick()
        val clipboard=ui.activity.getSystemService(android.content.ClipboardManager::class.java)
        ui.runOnIdle { assertEquals("WIFI:T:WPA;S:Synthetic;P:synthetic-secret;;",clipboard.primaryClip!!.getItemAt(0).text.toString()); assertTrue(clipboard.primaryClipDescription!!.extras!!.getBoolean("android.content.extra.IS_SENSITIVE")) }
    }
    @Test fun inline_utilities_preserve_exact_values_and_math_disables_active_content() {
        show(UtilityContent("calculation",display="0.333333",copy="0.3333333333333333",rich=RichText("1 / 3")))
        ui.onNodeWithText("Open calculation").assertDoesNotExist()
        ui.onNodeWithContentDescription("Calculation. 1 divided by 3 equals 0.333333").assertIsDisplayed()
        ui.onNodeWithContentDescription("Copy calculation").assertDoesNotExist()
        ui.runOnIdle { utility=UtilityContent("conversion",display="5 km",alternate="3.1069 mi",copy="3.1068559611866697 mi") }
        ui.onNodeWithContentDescription("Conversion. 5 km is 3.11 mi").assertIsDisplayed()
        ui.runOnIdle { utility=UtilityContent("math",display="\\frac{1}{2}",copy="\\frac{1}{2}",math=MathTypeset(1000f,1f,1f,.5f,emptyMap(),emptyList(),listOf(MathRule(0f,-.3f,1f,.05f,null)))) }
        ui.onNodeWithContentDescription("Formula. \\frac{1}{2}").assertIsDisplayed()
        // Typeset math is drawn by Compose: no web renderer exists at all.
        fun views(view: View): List<WebView> = if(view is WebView) listOf(view) else if(view is ViewGroup) (0 until view.childCount).flatMap { views(view.getChildAt(it)) } else emptyList()
        ui.runOnIdle { assertTrue(WindowInspector.getGlobalWindowViews().flatMap(::views).isEmpty()) }
        ui.onNodeWithTag("composer").assertIsDisplayed()
    }
}
