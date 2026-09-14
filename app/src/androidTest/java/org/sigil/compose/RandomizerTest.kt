package org.sigil.compose

import android.view.TextureView
import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class RandomizerTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    private fun textures(v:View):List<TextureView> = when(v) {is TextureView->listOf(v);is ViewGroup->(0 until v.childCount).flatMap {textures(v.getChildAt(it))};else->emptyList()}
    private fun positions():List<Pair<Int,Int>> {var result=emptyList<Pair<Int,Int>>();ui.runOnUiThread {result=textures(ui.activity.window.decorView).map {v->val p=IntArray(2);v.getLocationOnScreen(p);p[0] to p[1]}};return result}
    private fun ready(count:Int,flight:Boolean=false) {ui.waitForIdle();if(flight)ui.waitUntil(15_000) {ui.onAllNodesWithTag("material-flight-recorded").fetchSemanticsNodes().size==count};ui.waitUntil(15_000) {var ok=false;ui.runOnUiThread {val views=textures(ui.activity.window.decorView);ok=views.size==count&&views.all {it.alpha==1f&&it.isAvailable}};ok};android.os.SystemClock.sleep(100)}
    private fun capture(name:String) {val i=androidx.test.platform.app.InstrumentationRegistry.getInstrumentation();i.uiAutomation.takeScreenshot()?.let {b->java.io.File(ui.activity.cacheDir,"$name.png").outputStream().use {b.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)};b.recycle()}}
    @Test fun physical_motion_pause_placement_and_card_reveal_preserve_results() {
        val input=floatArrayOf(11f,18f,1f,1f,1f,11f,6f,5f,8f,14f,0f,3f,6f,5f)
        val recorded=MaterialNative.record(input)
        assertNotNull("A real trajectory must be available",recorded)
        assertArrayEquals(recorded,MaterialNative.record(input),0f)
        val flight=org.sigil.Flight(recorded!!,1,1f,Rect.Zero,Rect.Zero)
        assertEquals("Playback retains the physical simulation's timing",(flight.frames-1)*1000f/60,flight.duration.toFloat(),1f)
        val restStart=(recorded[0]+1)/(flight.frames-1)
        val restEnd=(recorded[0]+49)/(flight.frames-1)
        assertArrayEquals("Rest must preserve position and orientation",flight.pose(0,restStart),flight.pose(0,restEnd),.0001f)
        assertNull(MaterialNative.record(floatArrayOf(Float.NaN)))
        var p by mutableFloatStateOf(0f);var generation by mutableIntStateOf(0)
        var value by mutableStateOf(RandomizerMotion("dice",dice=listOf(DieFace(6,5)),result="5"))
        val scene=MaterialTimeline()
        ui.runOnUiThread {ui.activity.setSigilContent {
            val anchor=remember(value) {MaterialAnchor(value,true,{p},{generation},{p=1f}).also {a->
                val r=scene.viewport
                if(r.width>0)a.bounds=Rect(r.left+r.width*.45f,r.top+r.height*.65f,r.right-32,r.top+r.height*.85f)
            }}
            DisposableEffect(anchor) {scene.anchors["synthetic"]=anchor;onDispose {scene.anchors.remove("synthetic")}}
            Box(Modifier.fillMaxSize().onGloballyPositioned {
                val r=it.boundsInWindow();scene.viewport=r
                anchor.bounds=Rect(r.left+r.width*.45f,r.top+r.height*.65f,r.right-32,r.top+r.height*.85f)
                scene.bubbles["synthetic"]=Rect(r.left,r.top+r.height*.32f,r.left+r.width*.5f,r.top+r.height*.43f)
            }) {MaterialTimelineOverlay(scene,Modifier.matchParentSize())}
        }}
        ready(1,true)
        ui.runOnUiThread {
            val r=scene.viewport;val a=scene.anchors.getValue("synthetic").bounds
            val unit=38*ui.activity.resources.displayMetrics.density
            val o=scene.bubbles.getValue("synthetic")
            val sample=floatArrayOf(r.width/unit,r.height/unit,1f,1f,1f,(value.hashCode()and 0x7fffff).toFloat(),6f,5f,(a.center.x-r.left)/unit,(a.center.y-r.top)/unit,(o.left-r.left)/unit,(o.top-r.top)/unit,(o.right-r.left)/unit,(o.bottom-r.top)/unit)
            assertNotNull("The timeline trajectory must exist for ${sample.toList()}",MaterialNative.record(sample))
        }
        ui.runOnUiThread {p=.25f};ready(1,true);val moving=positions();capture("object-moving")
        ui.runOnUiThread {p=1f};ready(1,true);val placed=positions();assertNotEquals(moving,placed);capture("object-placed")
        ui.runOnUiThread {generation++;p=0f};ready(1,true)
        ui.runOnUiThread {p=.25f};ready(1,true);assertEquals("Replay follows the same recorded path",moving,positions())
        ui.runOnUiThread {p=1f};ready(1,true);assertEquals(placed,positions())
        ui.runOnUiThread {generation++;p=.25f};ready(1,true)
        ui.mainClock.autoAdvance=false
        val before=positions().single()
        ui.runOnUiThread {val a=scene.anchors.getValue("synthetic");a.bounds=a.bounds.translate(0f,20f)}
        ui.mainClock.advanceTimeBy(160);ui.waitForIdle();val between=positions().single()
        ui.mainClock.advanceTimeBy(400);ui.waitForIdle();val after=positions().single()
        assertEquals("Scrolling must follow the row immediately",before.first to before.second+20,between)
        assertEquals("Scrolling must not produce a delayed return animation",between,after)
        assertEquals("Scrolling must preserve playback",.25f,p,0f)
        ui.mainClock.autoAdvance=true
        ui.runOnUiThread {value=RandomizerMotion("choice",frames=listOf("Museum","Bookstore","Cafe"),selected=1,result="Bookstore");p=.4f}
        ui.waitForIdle()
        ui.runOnUiThread {assertEquals("Switching objects must preserve the requested preview position",.4f,p,0f)}
        ready(7);capture("picker-fan")
        assertEquals(1,ui.onAllNodesWithTag("card-mist-front").fetchSemanticsNodes().size)
        assertEquals(1,ui.onAllNodesWithTag("card-mist-back").fetchSemanticsNodes().size)
        fun cardPosition()=ui.onNodeWithTag("picker-card-3").fetchSemanticsNode().boundsInRoot.center
        ui.runOnUiThread {p=.88f};ready(1);val earlyCard=cardPosition();capture("picker-docking-early")
        ui.runOnUiThread {p=.93f};ready(1);val lateCard=cardPosition();capture("picker-docking-late")
        assertNotEquals("The chosen card must travel into its row",earlyCard,lateCard)
        ui.runOnUiThread {p=1f};ready(1);capture("picker-chosen")
        assertNotEquals(lateCard,cardPosition())
        assertEquals("Bookstore",value.result)
        ui.runOnUiThread {generation++;p=.4f};ready(7)
        ui.runOnUiThread {p=1f};ready(1);assertEquals("Bookstore",value.result)
        ui.runOnUiThread {value=RandomizerMotion("coin",frames=listOf("Heads","Tails"),selected=1,result="Tails");p=0f}
        ready(1,true);ui.runOnUiThread {p=.15f};ready(1,true);capture("coin-moving")
        var coinBitmap:android.graphics.Bitmap?=null
        ui.runOnUiThread {coinBitmap=textures(ui.activity.window.decorView).single().bitmap}
        ui.runOnUiThread {p=1f};ready(1,true);capture("coin-placed");assertEquals(1,value.selected)
        ui.runOnUiThread {val end=textures(ui.activity.window.decorView).single().bitmap!!;assertFalse("The coin must visibly flip before resting",end.sameAs(coinBitmap));end.recycle();coinBitmap?.recycle()}
    }
}
