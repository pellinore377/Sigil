package org.sigil.compose

import android.view.TextureView
import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.compose.runtime.*
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class MaterialMessageTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    private fun textures(view:View):List<TextureView> = when(view) {
        is TextureView -> listOf(view)
        is ViewGroup -> (0 until view.childCount).flatMap {textures(view.getChildAt(it))}
        else -> emptyList()
    }
    private fun screenshot(name:String) {
        val instrumentation=androidx.test.platform.app.InstrumentationRegistry.getInstrumentation()
        instrumentation.uiAutomation.takeScreenshot()?.let { bitmap ->
            java.io.File(instrumentation.targetContext.cacheDir,name).outputStream().use {bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
            bitmap.recycle()
        }
    }
    @Test fun more_than_the_gpu_capacity_settles_to_exact_bounded_snapshots() {
        val cache=ImageCache()
        var shown by mutableStateOf(true)
        ui.runOnUiThread {ui.activity.setSigilContent {
            CompositionLocalProvider(LocalImageCache provides cache) {
                if(shown)Column {repeat(5) {row->Row {repeat(5) {column->
                    val index=row*5+column
                    MaterialObject(0,30,index+1,null,null,Modifier.size(48.dp).testTag("settled-$index"))
                }}}}
            }
        }}
        ui.waitUntil(30_000) {var complete=false;ui.runOnUiThread {complete=cache.retainedMaterials()==25 && textures(ui.activity.window.decorView).isEmpty()};complete}
        fun signature(index:Int):Int {
            val image=ui.onNodeWithTag("settled-$index",useUnmergedTree=true).captureToImage()
            val pixels=IntArray(image.width*image.height);image.readPixels(pixels)
            assertTrue("Object $index must contain rendered detail",pixels.toSet().size>8)
            return pixels.contentHashCode()
        }
        val before=(0 until 25).map(::signature)
        ui.runOnIdle {shown=false};ui.waitForIdle()
        ui.runOnIdle {shown=true};ui.waitForIdle()
        ui.runOnUiThread {assertTrue("Re-entering settled rows must not allocate GPU views",textures(ui.activity.window.decorView).isEmpty())}
        assertEquals(before,(0 until 25).map(::signature))
        assertTrue(cache.retainedBytes()<=24*1024*1024)
    }
    @Test fun a_departing_row_releases_capacity_for_a_waiting_object() {
        var first by mutableIntStateOf(0)
        var failures=0
        ui.runOnUiThread {ui.activity.setSigilContent {
            Column {for(index in first until 19) key(index) {
                AndroidView(factory={context->MessageMaterialView(context,32){failures++}},
                    update={it.update(MaterialFrame(0,6,index%6+1,0,0xff706080.toInt(),0,1f,transparent=true),true)},
                    onRelease={it.close()},modifier=Modifier.size(20.dp))
            }}
        }}
        ui.waitUntil(30_000) {var count=0;ui.runOnUiThread {count=textures(ui.activity.window.decorView).count {it.alpha==1f}};count==18}
        ui.runOnUiThread {assertEquals("Capacity is temporary, not a renderer failure",0,failures);first=1}
        ui.waitUntil(15_000) {var ready=false;ui.runOnUiThread {val views=textures(ui.activity.window.decorView);ready=views.size==18 && views.all {it.alpha==1f}};ready}
        ui.runOnUiThread {assertEquals(0,failures)}
    }
    @Test fun settled_objects_keep_their_pose_and_row_offset_after_scrolling() {
        val chat=ChatSummary("self","Sample","","",true,emptyList())
        val dice=MessagePart("dice","dice","Dice",utility=UtilityContent("dice",display="7",motion=RandomizerMotion("dice",dice=listOf(DieFace(8,7)),result="7")))
        val result=ChatMessage("result","sam","",true,"9:33","sent",false,emptyList(),emptyList(),null,true,timestamp=2000,parts=listOf(dice))
        val messages=listOf(result)+(1..30).map {result.copy(id="text-$it",text="An earlier synthetic message $it",parts=emptyList(),timestamp=2000-it.toLong())}
        ui.runOnUiThread {ui.activity.setSigilContent {SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self",timelineLoaded=true,messages=messages),{_,_->})}}
        fun signature():Int? {var value:Int?=null;ui.runOnUiThread {textures(ui.activity.window.decorView).firstOrNull()?.bitmap?.let {b->val pixels=IntArray(b.width*b.height);b.getPixels(pixels,0,b.width,0,0,b.width,b.height);value=pixels.contentHashCode();b.recycle()}};return value}
        ui.waitUntil(15_000){signature()!=null}
        android.os.SystemClock.sleep(300)
        val before=signature()
        val objectBounds=ui.onNodeWithTag("material-object-0",useUnmergedTree=true).fetchSemanticsNode().boundsInWindow
        val row=ui.onNodeWithContentDescription("Dice: d8 · 7").fetchSemanticsNode().boundsInWindow
        ui.onNodeWithTag("timeline").performScrollToIndex(20)
        ui.onNodeWithTag("material-object-0",useUnmergedTree=true).assertDoesNotExist()
        ui.onNodeWithTag("timeline").performScrollToIndex(0)
        ui.waitUntil(15_000){signature()!=null}
        android.os.SystemClock.sleep(300)
        val after=ui.onNodeWithTag("material-object-0",useUnmergedTree=true).fetchSemanticsNode().boundsInWindow
        val afterRow=ui.onNodeWithContentDescription("Dice: d8 · 7").fetchSemanticsNode().boundsInWindow
        assertEquals(objectBounds.left-row.left,after.left-afterRow.left,1f)
        assertEquals(objectBounds.top-row.top,after.top-afterRow.top,1f)
        assertEquals("Scrolling must not rotate or redraw a different result",before,signature())
    }
    @Test fun replay_from_the_message_menu_runs_the_timeline_animation() = replay(
        RandomizerMotion("dice",dice=listOf(DieFace(6,5)),result="5"),"Dice: d6 · 5")
    @Test fun card_menu_replay_has_a_fan_mist_and_animated_placement() = replay(
        RandomizerMotion("choice",frames=listOf("Museum","Bookstore","Cafe"),selected=1,result="Bookstore"),"Chosen: Bookstore")
    @Test fun coin_menu_replay_visibly_flips() = replay(
        RandomizerMotion("coin",frames=listOf("Heads","Tails"),selected=1,result="Tails"),"Coin: Tails")
    private fun replay(motion:RandomizerMotion,description:String) {
        val chat=ChatSummary("self","Sample","","",true,emptyList())
        val part=MessagePart("card",if(motion.kind=="dice")"dice" else "pick","Result",utility=UtilityContent(if(motion.kind=="dice")"dice" else "pick",display=motion.result,motion=motion))
        val message=ChatMessage("replay","sam","",true,"9:33","sent",false,emptyList(),emptyList(),null,true,parts=listOf(part))
        val state=MessengerState(phase="connected",chats=listOf(chat),selected="self",timelineLoaded=true,messages=listOf(message))
        ui.runOnUiThread {ui.activity.setSigilContent {SigilApp(NativeCore::palette,NativeCore::analyze,state,{_,_->})}}
        ui.waitForIdle()
        ui.onNodeWithContentDescription(description).assertExists()
        ui.waitUntil(10_000) {ui.onAllNodesWithTag(if(motion.kind=="choice")"picker-card-3" else "material-object-0",useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()}
        ui.onNodeWithTag(if(motion.kind=="choice")"picker-card-3" else "material-object-0",useUnmergedTree=true).performTouchInput {longClick()}
        ui.waitForIdle()
        ui.waitUntil(10_000) {var ready=false;ui.runOnUiThread {
            val roots=if(android.os.Build.VERSION.SDK_INT>=29)android.view.inspector.WindowInspector.getGlobalWindowViews()else listOf(ui.activity.window.decorView)
            val views=roots.flatMap(::textures);ready=views.size>=(if(android.os.Build.VERSION.SDK_INT>=29)2 else 1) && views.all {it.alpha==1f}
        };ready}
        android.os.SystemClock.sleep(500)
        screenshot("${motion.kind}-menu.png")
        ui.onNodeWithText("Replay animation").performScrollTo()
        ui.mainClock.autoAdvance=false
        ui.onNodeWithText("Replay animation").performClick()
        ui.mainClock.advanceTimeBy(400);ui.waitForIdle()
        val positions=mutableListOf<Int>()
        val images=mutableSetOf<Int>()
        var fan=false
        repeat(100) {
            ui.mainClock.advanceTimeBy(80);ui.waitForIdle();android.os.SystemClock.sleep(35)
            ui.runOnUiThread {val views=textures(ui.activity.window.decorView);fan=fan||views.size==7;views.firstOrNull {it.alpha==1f}?.let {v->val p=IntArray(2);v.getLocationOnScreen(p);positions+=p[if(motion.kind=="choice")0 else 1]}}
            if(motion.kind=="coin")ui.runOnUiThread {textures(ui.activity.window.decorView).singleOrNull()?.bitmap?.let {b->val pixels=IntArray(b.width*b.height);b.getPixels(pixels,0,b.width,0,0,b.width,b.height);images+=pixels.contentHashCode();b.recycle()}}
            if(motion.kind=="choice" && it in listOf(5,12,20,28,36))screenshot("card-replay-$it.png")
        }
        if(motion.kind=="coin")assertTrue("The coin must show multiple rendered orientations",images.size>8)
        else assertTrue("The real replay action must move the object, observed $positions",positions.isNotEmpty() && positions.max()-positions.min()>150)
        if(motion.kind=="choice")assertTrue("Replaying a card must show the fan",fan)
    }
    @Test fun new_message_rolls_across_the_real_timeline() {
        val cache=ImageCache()
        val chat=ChatSummary("self","Sample","","",true,emptyList())
        val old=ChatMessage("old","sam","An earlier message",false,"9:33","sent",false,emptyList(),emptyList(),null,true,timestamp=1000)
        val part=MessagePart("card","dice","Dice",utility=UtilityContent("dice",display="5",motion=RandomizerMotion("dice",dice=listOf(DieFace(6,5)),result="5")))
        var state by mutableStateOf(MessengerState(phase="connected",chats=listOf(chat),selected="self",timelineLoaded=true,messages=listOf(old)))
        ui.runOnUiThread {ui.activity.setSigilContent {CompositionLocalProvider(LocalImageCache provides cache) {SigilApp(NativeCore::palette,NativeCore::analyze,state,{_,_->})}}}
        ui.waitForIdle()
        ui.mainClock.autoAdvance=false
        ui.runOnUiThread {state=state.copy(messages=listOf(old.copy(id="new",mine=true,parts=listOf(part),timestamp=2000),old))}
        val positions=mutableListOf<Int>()
        repeat(100) {
            ui.mainClock.advanceTimeBy(80)
            ui.waitForIdle()
            android.os.SystemClock.sleep(40)
            ui.runOnUiThread {textures(ui.activity.window.decorView).firstOrNull {it.alpha==1f}?.let {v->val p=IntArray(2);v.getLocationOnScreen(p);positions+=p[1]}}
            if(it in listOf(8,12,18,25))screenshot("roll-$it.png")
        }
        screenshot("new-message.png")
        assertTrue("A fresh message must visibly travel through the timeline; observed $positions",positions.isNotEmpty() && positions.max()-positions.min()>200)
        ui.mainClock.autoAdvance=true
        ui.waitUntil(15_000) {var retired=false;ui.runOnUiThread {retired=cache.retainedMaterials()==1 && textures(ui.activity.window.decorView).isEmpty()};retired}
    }
    @Test fun stored_results_render_inside_messages_without_an_expanded_viewer() {
        assertTrue("Native material library must be packaged",MaterialNative.available)
        val chat=ChatSummary("self","@sample:example.test","","",true,emptyList())
        val faces=listOf(DieFace(4,3),DieFace(6,5),DieFace(8,7),DieFace(10,9),DieFace(12,11),DieFace(20,19))
        val motion=RandomizerMotion("dice",dice=faces,result="54")
        val part=MessagePart("card","dice","Dice",utility=UtilityContent("dice",display="54",motion=motion))
        val message=ChatMessage("materials","sam","Synthetic dice",true,"9:33","sent",false,emptyList(),emptyList(),null,true,timestamp=1000,parts=listOf(part))
        var state by mutableStateOf(MessengerState(phase="connected",chats=listOf(chat),selected="self",timelineLoaded=true,messages=listOf(message)))
        ui.runOnUiThread {ui.activity.setSigilContent {SigilApp(NativeCore::palette,NativeCore::analyze,state,{_,_->})}}
        ui.waitUntil(20_000) {
            var ready=false
            ui.runOnUiThread {
                val views=textures(ui.activity.window.decorView)
                ready=views.size==6 && views.all {view->
                    val bitmap=view.bitmap
                    val valid=view.alpha==1f && bitmap!=null && bitmap.width>8 && bitmap.height>8 && bitmap.getPixel(bitmap.width/2,bitmap.height/2)!=bitmap.getPixel(1,1)
                    bitmap?.recycle();valid
                }
            }
            ready
        }
        screenshot("material-dice.png")
        ui.onNodeWithContentDescription("Dice: d4 · 3, d6 · 5, d8 · 7, d10 · 9, d12 · 11, d20 · 19").assertIsDisplayed()
        ui.onNodeWithText("Open dice").assertDoesNotExist()
        val coin=part.copy(utility=UtilityContent("pick",display="Tails",motion=RandomizerMotion("coin",frames=listOf("Heads","Tails"),selected=1,result="Tails")))
        ui.runOnUiThread {state=state.copy(messages=listOf(message.copy(parts=listOf(coin))))}
        ui.onNodeWithContentDescription("Coin: Tails").assertIsDisplayed()
        ui.onNodeWithText("Open coin flip").assertDoesNotExist()
        ui.waitUntil(10_000) {var count=0;ui.runOnUiThread {count=textures(ui.activity.window.decorView).count {it.alpha==1f}};count==1}
    }
}
