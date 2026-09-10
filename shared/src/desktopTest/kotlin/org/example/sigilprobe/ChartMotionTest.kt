package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class ChartMotionTest {
    @get:Rule val ui=createComposeRule()
    @Test fun all_chart_reveals_preserve_geometry_values_and_settle_to_the_static_plot() {
        val clock=TextPlayback(true)
        var kind by mutableStateOf("bar")
        var horizontal by mutableStateOf(false)
        var reduced by mutableStateOf(false)
        var expanded by mutableStateOf(false)
        var disabled by mutableStateOf(false)
        var selected:Int?=null
        val data=ChartContent("bar",RichText("Synthetic"),false,.5f,listOf("-4","-2","0","2","4"),emptyList(),"A\t-2\nB\t3",
            listOf(ChartPoint(RichText("A"),.25f,.25f,"-2",null,.25f,"25"),ChartPoint(RichText("B"),.75f,.875f,"3",null,.75f,"75")))
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalMotion provides MotionPolicy(reduced),LocalAppearance provides Appearance(messageEffects=!disabled)) {
            MessageMotion("synthetic",clock,false) {
                ChartPlot(data.copy(kind=kind,horizontal=horizontal),emptySet(),null,{selected=it},Modifier.size(300.dp,240.dp).testTag("plot"),zoomable=expanded)
            }
        }}}
        fun pixels():List<androidx.compose.ui.graphics.Color> {
            val image=ui.onNodeWithTag("plot").captureToImage().toPixelMap()
            return buildList {repeat(image.height) {y->repeat(image.width) {x->add(image[x,y])}}}
        }
        for(type in listOf("bar","horizontal","line","area","scatter","pie","donut")) {
            ui.runOnIdle {kind=if(type=="horizontal")"bar" else type;horizontal=type=="horizontal";clock.elapsed=250f}
            val moving=pixels()
            val bounds=ui.onNodeWithTag("plot").fetchSemanticsNode().boundsInRoot
            ui.runOnIdle {clock.elapsed=2000f}
            val settled=pixels()
            assertNotEquals(moving,settled,type)
            assertEquals(bounds,ui.onNodeWithTag("plot").fetchSemanticsNode().boundsInRoot)
            ui.onNodeWithContentDescription("${kind.replaceFirstChar {it.uppercase()}} chart, 2 points. Values are listed below.").assertExists()
            ui.runOnIdle {clock.replay();clock.elapsed=250f}
            assertEquals(moving,pixels(),"Replay must be deterministic: $type")
            ui.runOnIdle {reduced=true}
            assertEquals(settled,pixels(),"Reduced motion: $type")
            ui.runOnIdle {reduced=false;disabled=true;clock.elapsed=250f}
            assertEquals(settled,pixels(),"Effects disabled: $type")
            ui.runOnIdle {disabled=false}
        }
        ui.runOnIdle {kind="bar";horizontal=false;expanded=true;clock.elapsed=250f}
        val viewer=pixels()
        ui.runOnIdle {clock.elapsed=2000f}
        assertEquals(viewer,pixels(),"Expanded viewers stay settled")
        assertNull(selected)
        assertEquals("A\t-2\nB\t3",data.copyData)
    }
}
