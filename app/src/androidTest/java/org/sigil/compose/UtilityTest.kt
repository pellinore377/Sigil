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
    @Test fun expanded_utilities_preserve_exact_values_and_math_disables_active_content() {
        show(UtilityContent("calculation",display="0.333333",copy="0.3333333333333333",rich=RichText("1 / 3")))
        ui.onNodeWithText("Open calculation").performClick()
        ui.onNodeWithContentDescription("Copy calculation").performClick()
        val clipboard=ui.activity.getSystemService(android.content.ClipboardManager::class.java)
        ui.runOnIdle { assertEquals("0.3333333333333333",clipboard.primaryClip!!.getItemAt(0).text.toString()) }
        ui.onNodeWithContentDescription("Close calculation").performClick()
        ui.runOnIdle { utility=UtilityContent("conversion",display="5 km",alternate="3.1069 mi",copy="3.1068559611866697 mi") }
        ui.onNodeWithText("Swap display").performClick()
        val input=ui.onNodeWithText("5 km", useUnmergedTree=true).fetchSemanticsNode().boundsInRoot
        val output=ui.onNodeWithText("3.1069 mi", useUnmergedTree=true).fetchSemanticsNode().boundsInRoot
        assertTrue(output.top<input.top)
        ui.runOnIdle { utility=UtilityContent("math",display="\\frac{1}{2}",copy="\\frac{1}{2}",mathml="<math xmlns='http://www.w3.org/1998/Math/MathML'><mfrac><mn>1</mn><mn>2</mn></mfrac></math>") }
        ui.onNodeWithText("Open formula").performClick()
        fun views(view: View): List<WebView> = if(view is WebView) listOf(view) else if(view is ViewGroup) (0 until view.childCount).flatMap { views(view.getChildAt(it)) } else emptyList()
        ui.waitUntil(10_000) { var loaded=false; ui.runOnUiThread { loaded=WindowInspector.getGlobalWindowViews().flatMap(::views).any { it.progress==100 } }; loaded }
        ui.runOnIdle {
            val renderers=WindowInspector.getGlobalWindowViews().flatMap(::views)
            assertTrue(renderers.isNotEmpty())
            renderers.forEach { view ->
                assertFalse(view.settings.javaScriptEnabled); assertFalse(view.settings.allowFileAccess)
                assertFalse(view.settings.allowContentAccess); assertFalse(view.settings.domStorageEnabled)
                assertTrue(view.settings.blockNetworkLoads); assertTrue(view.settings.blockNetworkImage)
            }
        }
        ui.onNode(isDialog()).captureToImage().asAndroidBitmap().let { bitmap -> java.io.File(ui.activity.cacheDir,"utility-math.png").outputStream().use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it) } }
        ui.onNodeWithContentDescription("Close formula").performClick()
        ui.onNodeWithTag("composer").assertIsDisplayed()
    }
}
