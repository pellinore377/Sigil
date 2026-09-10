package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.runtime.*
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class ChartTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    @Test fun new_chart_reveals_once_and_replays_without_changing_its_values() {
        val chart=ChartContent("bar",RichText("Synthetic animated chart"),false,.5f,listOf("-4","-2","0","2","4"),emptyList(),"A\t-2\nB\t3",
            listOf(ChartPoint(RichText("A"),.25f,.25f,"-2",null,.25f,"25"),ChartPoint(RichText("B"),.75f,.875f,"3",null,.75f,"75")))
        val chat=ChatSummary("self","@sam:example.test","","",true,emptyList())
        var state by mutableStateOf(MessengerState(phase="connected",chats=listOf(chat),selected="self",timelineLoaded=true))
        ui.mainClock.autoAdvance=false
        ui.runOnUiThread {ui.activity.setSigilContent {SigilApp(NativeCore::palette,NativeCore::analyze,state,{_,_->})}}
        ui.mainClock.advanceTimeBy(600)
        val message=ChatMessage("chart","sam","Chart",true,"9:33","sent",false,emptyList(),emptyList(),null,true,
            timestamp=1000,parts=listOf(MessagePart("card","chart","Chart",chart=chart)))
        ui.runOnUiThread {state=state.copy(messages=listOf(message))}
        ui.mainClock.advanceTimeBy(280)
        val plot=ui.onNodeWithContentDescription("Bar chart, 2 points. Values are listed below.")
        val moving=plot.captureToImage().asAndroidBitmap()
        val bounds=plot.fetchSemanticsNode().boundsInRoot.size
        ui.mainClock.advanceTimeBy(1200)
        val settled=plot.captureToImage().asAndroidBitmap()
        assertFalse(moving.sameAs(settled))
        assertEquals(bounds,plot.fetchSemanticsNode().boundsInRoot.size)
        ui.onNodeWithText("Synthetic animated chart",useUnmergedTree=true).assertIsDisplayed().performTouchInput {longClick()}
        ui.mainClock.advanceTimeBy(500)
        ui.onNode(isDialog()).assertExists()
        ui.mainClock.autoAdvance=true
        ui.onNodeWithText("Replay animation").performScrollTo()
        ui.mainClock.autoAdvance=false
        ui.onNodeWithText("Replay animation").performClick()
        ui.mainClock.advanceTimeBy(500)
        assertFalse(plot.captureToImage().asAndroidBitmap().sameAs(settled))
        ui.mainClock.advanceTimeBy(1200)
        assertTrue(plot.captureToImage().asAndroidBitmap().sameAs(settled))
        ui.onNodeWithText("-2").assertExists()
        ui.onNodeWithText("3").assertExists()
    }
    @Test fun chart_types_expand_select_values_toggle_points_and_copy_data() {
        var kind by mutableStateOf("pie")
        val chart = ChartContent("pie", RichText("Synthetic chart"), false, 0f, listOf("0", "1", "2", "3", "4"), listOf("0", "1", "2", "3", "4"), "A\t1\nB\t3",
            listOf(ChartPoint(RichText("A"), .25f, .25f, "1", null, .25f, "25"), ChartPoint(RichText("B"), .75f, .75f, "3", null, .75f, "75")))
        val chat = ChatSummary("self", "@sam:example.test", "", "", true, emptyList())
        ui.runOnUiThread { ui.activity.setSigilContent {
            val value = chart.copy(kind = kind, horizontal = kind == "bar")
            val message = ChatMessage("chart", "sam", "Chart", true, "9:33", "sent", false, emptyList(), emptyList(), null, true,
                timestamp = 1000, parts = listOf(MessagePart("card", "chart", "Chart", chart = value)))
            SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), selected = "self", messages = listOf(message)), { _, _ -> })
        } }
        for (type in listOf("pie", "donut", "bar", "line", "area", "scatter")) {
            ui.runOnIdle { kind = type }
            ui.onNodeWithText("Open chart · 2 points").performClick()
            val plot = ui.onNode(hasContentDescription("${type.replaceFirstChar { it.uppercase() }} chart, 2 points. Values are listed below.") and hasAnyAncestor(isDialog()))
            plot.assertIsDisplayed()
            plot.performTouchInput {
                if (type in listOf("pie", "donut")) { val radius = minOf(width, height) * .36f; click(center + Offset(-radius * .7f, radius * .7f)) }
                else click(Offset(width * .75f, height * if (type == "bar") .75f else .25f))
            }
            ui.onNodeWithContentDescription("Selected point 2").assertIsDisplayed()
            plot.performTouchInput {
                down(0, center - Offset(width * .1f, 0f)); down(1, center + Offset(width * .1f, 0f))
                repeat(8) { i -> moveTo(0, center - Offset(width * (.1f + .015f * (i + 1)), 0f), delayMillis = 16); moveTo(1, center + Offset(width * (.1f + .015f * (i + 1)), 0f), delayMillis = 16) }
                up(0); up(1)
            }
            ui.onNodeWithText("Fit chart").assertIsEnabled().performClick().assertIsNotEnabled()
            ui.onNodeWithContentDescription("Hide point 1").performClick()
            ui.onNodeWithContentDescription("Show point 1").assertIsDisplayed().performClick()
            ui.onNodeWithContentDescription("Copy chart data").performClick()
            ui.runOnIdle { assertEquals(chart.copyData, ui.activity.getSystemService(android.content.ClipboardManager::class.java).primaryClip!!.getItemAt(0).text.toString()) }
            ui.onNode(isDialog()).captureToImage().asAndroidBitmap().let { bitmap -> java.io.File(ui.activity.cacheDir, "chart-$type.png").outputStream().use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) } }
            ui.onNodeWithContentDescription("Close chart").performClick()
        }
        ui.onNodeWithTag("composer").assertIsDisplayed()
    }
}
