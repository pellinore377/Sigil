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

class RandomizerMotionTest {
    @get:Rule val ui=createComposeRule()
    @Test fun moving_cards_request_motion_rendering_then_refine_when_settled() {
        var progress by mutableStateOf(.4f)
        val rendered=mutableMapOf<Int,Float>()
        val platform=object:MaterialPlatform {
            override val available=true
            @Composable override fun Object(kind:Int,sides:Int,face:Int,rotation:FloatArray?,label:String?,modifier:Modifier,progress:Float) {
                SideEffect {rendered[kind]=progress}
                Box(modifier)
            }
            override suspend fun record(data:FloatArray):FloatArray?=null
            override fun horizontalExtent(sides:Int,face:Int,rotation:FloatArray?,outgoing:Boolean)=1f
        }
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalMaterialPlatform provides platform) {
            PickerCards(RandomizerMotion("choice",frames=listOf("One","Two"),selected=0,result="One"),progress,Modifier.size(500.dp))
        }}}
        ui.runOnIdle {assertTrue(rendered.getValue(2)<1f);progress=1f}
        ui.runOnIdle {assertEquals(1f,rendered.getValue(2))}
    }
    @Test fun launch_preserves_preview_rows_in_a_wider_timeline() {
        val viewport=androidx.compose.ui.geometry.Rect(0f,0f,900f,900f)
        val preview=androidx.compose.ui.geometry.Rect(200f,500f,514f,708f)
        val positions=previewObjectOrigins(viewport,preview,6,102f,4f)
        assertEquals(listOf(251f,357f,463f,251f,357f,463f),positions.map {it.x})
        assertEquals(listOf(551f,551f,551f,657f,657f,657f),positions.map {it.y})
    }
    @Test fun preview_launch_objects_fit_the_viewport_without_overlapping() {
        for(width in listOf(320f,700f))for(count in 1..6)for(center in listOf(10f,width/2,width-10f)) {
            val viewport=androidx.compose.ui.geometry.Rect(20f,40f,20f+width,600f)
            val origin=androidx.compose.ui.geometry.Rect(center,520f,center+20f,580f)
            val points=previewObjectOrigins(viewport,origin,count,102f,4f)
            assertEquals(count,points.size)
            for(p in points) {
                assertTrue(p.x>=51f && p.x+51f<=viewport.width)
                assertTrue(p.y>=51f && p.y+51f<=viewport.height)
            }
            points.forEachIndexed {i,a->points.drop(i+1).forEach {b->assertTrue(kotlin.math.abs(a.x-b.x)>=106f || kotlin.math.abs(a.y-b.y)>=106f)}}
        }
    }
    @Test fun pending_physics_does_not_mount_a_static_object_at_the_destination() {
        val ready=kotlinx.coroutines.CompletableDeferred<Unit>()
        val started=java.util.concurrent.atomic.AtomicBoolean(false)
        val timeline=MaterialTimeline().apply {viewport=androidx.compose.ui.geometry.Rect(0f,0f,600f,500f)}
        val anchor=MaterialAnchor(RandomizerMotion("dice",listOf(DieFace(6,4))),true,{0f},{0}).apply {bounds=androidx.compose.ui.geometry.Rect(400f,100f,590f,240f)}
        timeline.anchors["message"]=anchor
        val platform=object:MaterialPlatform {
            override val available=true
            @Composable override fun Object(kind:Int,sides:Int,face:Int,rotation:FloatArray?,label:String?,modifier:Modifier,progress:Float) {Box(modifier) {Box(Modifier.fillMaxSize().testTag("mounted-material"))}}
            override suspend fun record(data:FloatArray):FloatArray? {started.set(true);ready.await();return null}
            override fun horizontalExtent(sides:Int,face:Int,rotation:FloatArray?,outgoing:Boolean)=1.14f
        }
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalMaterialPlatform provides platform) {MaterialTimelineOverlay(timeline,Modifier.size(600.dp,500.dp))}}}
        ui.waitUntil(5000) {started.get()}
        ui.onNodeWithTag("mounted-material").assertDoesNotExist()
        ready.complete(Unit)
        ui.waitUntil(5000) {ui.onAllNodesWithTag("mounted-material").fetchSemanticsNodes().isNotEmpty()}
    }
    @Test fun stored_results_replay_deterministically_and_obey_motion_and_lifecycle_preferences() {
        val examples=listOf(
            RandomizerMotion("dice",listOf(DieFace(4,3),DieFace(6,5),DieFace(8,7),DieFace(10,9),DieFace(12,11),DieFace(20,17))),
            RandomizerMotion("dice",listOf(DieFace(100,87))),
            RandomizerMotion("coin",frames=listOf("Heads","Tails"),selected=0,result="Heads"),
            RandomizerMotion("coin",frames=listOf("Heads","Tails"),selected=1,result="Tails"),
            RandomizerMotion("choice",frames=listOf("Pizza","Tacos","Pasta"),selected=1,result="Tacos"),
            RandomizerMotion("number",frames=listOf("8","12","1","9"),result="7"))
        var value by mutableStateOf(examples.first())
        var reduced by mutableStateOf(false)
        var full by mutableStateOf(false)
        var visible by mutableStateOf(true)
        val clock=TextPlayback()
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalMotion provides MotionPolicy(reduced),LocalMotionVisible provides visible) {
            MessageMotion("random",clock,false) {Box(Modifier.width(280.dp).testTag("stage")) {RandomizerStage(value,full)}}
        }}}
        fun pixels():List<androidx.compose.ui.graphics.Color> {
            val pixels=ui.onNodeWithTag("stage").captureToImage().toPixelMap()
            return buildList {repeat(pixels.height) {y->repeat(pixels.width) {x->add(pixels[x,y])}}}
        }
        for(example in examples) {
            ui.runOnIdle {value=example;clock.elapsed=350f}
            val moving=pixels();val bounds=ui.onNodeWithTag("stage").fetchSemanticsNode().boundsInRoot.size
            ui.runOnIdle {clock.elapsed=12000f}
            val settled=pixels()
            assertNotEquals(moving,settled,example.kind)
            assertEquals(bounds,ui.onNodeWithTag("stage").fetchSemanticsNode().boundsInRoot.size)
            ui.runOnIdle {clock.replay();clock.elapsed=350f}
            assertEquals(moving,pixels(),"Replay ${example.kind}")
            ui.runOnIdle {reduced=true}
            assertEquals(settled,pixels(),"Reduced motion ${example.kind}")
            ui.runOnIdle {reduced=false;full=true;clock.elapsed=350f}
            assertEquals(settled,pixels(),"Expanded ${example.kind}")
            ui.runOnIdle {full=false;clock.elapsed=350f;visible=false}
            ui.runOnIdle {visible=true}
            assertEquals(settled,pixels(),"Resume must show the stored result")
            assertEquals(example,value)
        }
    }
}
