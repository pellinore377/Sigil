package org.sigil

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Rule
import org.junit.Test
import kotlin.math.pow
import kotlin.test.*

class ChartCardTest {
    @get:Rule val ui=createComposeRule()

    /** Ink and ground for incoming and outgoing bubbles in light and dark. */
    private fun grounds():List<Triple<String,Color,Color>> {
        val found=mutableMapOf<String,Pair<Color,Color>>()
        ui.setContent {Column {for (dark in listOf(false,true)) SigilTheme(Appearance(mode=if(dark)"Dark" else "Light"),palette=NativeCore::palette) {
            val scheme=MaterialTheme.colorScheme
            val mode=if(dark)"dark" else "light"
            val outInk=LocalOutgoingInk.current; val outGround=LocalOutgoingBubble.current
            SideEffect {
                found["in-$mode"]=scheme.onSurface to scheme.surfaceContainer
                found["out-$mode"]=outInk to outGround
            }
        }}}
        ui.waitForIdle()
        return found.map {(k,v)->Triple(k,v.first,v.second)}
    }

    @Test fun the_ink_ramp_keeps_categories_apart_on_every_bubble() {
        val cases=grounds()
        assertEquals(4,cases.map {it.first}.distinct().size)
        for ((name,ink,ground) in cases) for (count in 2..4) {
            val stops=chartColors(count,ink,ground)
            val total=chartContrast(ink,ground)
            assertEquals(count,stops.size)
            assertTrue(chartContrast(stops[0],ink)<1.02f,"$name/$count: the first stop is the ink")
            val steps=stops.zipWithNext().map {(a,b)->chartContrast(a,b)}
            val palest=chartContrast(stops.last(),ground)
            // Adjacent stops stay distinct; the palest reads as a mark on the ground wherever the ink leaves room.
            assertTrue(steps.all {it>=(if(count<4)1.58f else 1.5f)},"$name/$count adjacent $steps")
            if (total>=3.1f*1.6f.pow(count-1)) assertTrue(palest>=3f,"$name/$count palest $palest")
            else assertTrue(palest>=1.9f,"$name/$count palest $palest")
            if (name=="in-light"||name=="in-dark") assertTrue(steps.all {it>=1.7f}&&palest>=3f,"$name/$count $steps $palest")
        }
    }

    private fun point(value:String)=ChartPoint(RichText("A"),0f,0f,value,null,0f,"0")
    private fun chart(vararg values:String)=ChartContent("bar",RichText("Synthetic"),false,0f,emptyList(),emptyList(),null,values.map {point(it)})

    @Test fun all_zero_data_rests_on_a_zero_baseline() {
        val axis=chart("0","0","0").valueAxis()
        assertEquals("0",axis.ticks.first().second)
        assertEquals(0f,axis.zero)
        assertTrue(axis.positions.all {it==0f})
    }

    @Test fun negative_ticks_use_a_true_minus_sign() {
        val labels=chart("-2","16").valueAxis().ticks.map {it.second}
        assertEquals(listOf("−10","0","10","20"),labels)
        assertTrue(labels.none {'-' in it})
    }

    @Test fun axes_prefer_four_whole_ticks() {
        for ((low,high) in listOf(0.0 to 31.0,0.0 to 40.0,-2.0 to 16.0,0.0 to 7.0,0.0 to 1234.0,3.0 to 3.0)) {
            val scale=chartScale(low,high,whole=true)
            assertTrue(scale.ticks.size in 3..5,"$low..$high: ${scale.ticks}")
            assertTrue(scale.ticks.all {it==kotlin.math.floor(it)},"$low..$high: ${scale.ticks}")
            assertTrue(scale.min<=minOf(low,high)&&scale.max>=maxOf(low,high))
        }
        assertEquals(4,chartScale(-2.0,16.0).ticks.size)
    }
}
