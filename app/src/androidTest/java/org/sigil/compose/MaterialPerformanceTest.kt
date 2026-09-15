package org.sigil.compose

import android.view.TextureView
import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.*
import androidx.compose.ui.Modifier
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.unit.dp
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class MaterialPerformanceTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    private fun report(value:String) {androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().sendStatus(0,android.os.Bundle().apply {putString("stream","\n$value\n")})}
    private fun textures(v:View):List<TextureView> = when(v) {is TextureView->listOf(v);is ViewGroup->(0 until v.childCount).flatMap {textures(v.getChildAt(it))};else->emptyList()}
    @Test fun mixed_conversation_scroll_reports_frame_latency_and_restores_objects() = conversationScroll(true)
    @Test fun same_timeline_without_gpu_objects_reports_baseline() = conversationScroll(false)
    private fun conversationScroll(renderObjects:Boolean) {
        val platform=if(renderObjects)AndroidMaterials else object:MaterialPlatform by AndroidMaterials {
            @Composable override fun Object(kind:Int,sides:Int,face:Int,rotation:FloatArray?,label:String?,modifier:Modifier,progress:Float) {Box(modifier)}
        }
        val chat=ChatSummary("self","Sample","","",true,emptyList())
        val messages=(0 until 60).map {index->
            val motion=when(index%8) {
                0->RandomizerMotion("dice",dice=listOf(DieFace(6,5),DieFace(12,7),DieFace(20,13)),result="25")
                2->RandomizerMotion("coin",frames=listOf("Heads","Tails"),selected=1,result="Tails")
                4->RandomizerMotion("choice",frames=listOf("Museum","Park","Cafe"),selected=1,result="Park")
                else->null
            }
            val parts=motion?.let {listOf(MessagePart("object","dice","Result",utility=UtilityContent("dice",display=it.result,motion=it)))} ?: emptyList()
            ChatMessage("scroll-$index","sam",if(motion==null)"A synthetic conversation message $index with enough text to wrap naturally while scrolling through the timeline." else "",index%2==0,"9:33","sent",false,emptyList(),emptyList(),null,true,timestamp=2000-index.toLong(),parts=parts)
        }
        ui.runOnUiThread {ui.activity.setSigilContent {CompositionLocalProvider(LocalMaterialPlatform provides platform) {SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self",timelineLoaded=true,messages=messages),{_,_->})}}}
        if(renderObjects)ui.waitUntil(30_000) {var ready=false;ui.runOnUiThread {val views=textures(ui.activity.window.decorView);ready=views.isNotEmpty() && views.all {it.alpha==1f}};ready}
        ui.waitForIdle()
        val bounds=ui.onNodeWithTag("timeline").fetchSemanticsNode().boundsInWindow
        val automation=androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().uiAutomation
        fun swipe(down:Boolean) {
            val started=android.os.SystemClock.uptimeMillis()
            val x=bounds.center.x
            val from=bounds.top+bounds.height*(if(down).25f else .75f)
            val to=bounds.top+bounds.height*(if(down).75f else .25f)
            fun inject(action:Int,y:Float) {
                val event=android.view.MotionEvent.obtain(started,android.os.SystemClock.uptimeMillis(),action,x,y,0)
                try {check(automation.injectInputEvent(event,true))}finally {event.recycle()}
            }
            inject(android.view.MotionEvent.ACTION_DOWN,from)
            for(step in 1..36) {
                val remaining=started+step*600/36-android.os.SystemClock.uptimeMillis()
                if(remaining>0)android.os.SystemClock.sleep(remaining)
                inject(android.view.MotionEvent.ACTION_MOVE,from+(to-from)*step/36)
            }
            inject(android.view.MotionEvent.ACTION_UP,to)
            android.os.SystemClock.sleep(150)
        }
        val thread=android.os.HandlerThread("Timeline frame metrics").apply {start()}
        val frames=java.util.Collections.synchronizedList(mutableListOf<LongArray>())
        val listener=android.view.Window.OnFrameMetricsAvailableListener {_,metrics,_->frames.add(LongArray(9){metrics.getMetric(it)})}
        ui.runOnUiThread {ui.activity.window.addOnFrameMetricsAvailableListener(listener,android.os.Handler(thread.looper))}
        try {
            repeat(5) {swipe(true)}
            repeat(5) {swipe(false)}
        } finally {
            ui.runOnUiThread {ui.activity.window.removeOnFrameMetricsAvailableListener(listener)}
            thread.quitSafely();thread.join()
        }
        ui.onNodeWithTag("timeline").performScrollToIndex(0)
        if(renderObjects)ui.waitUntil(30_000) {var ready=false;ui.runOnUiThread {val views=textures(ui.activity.window.decorView);ready=views.isNotEmpty() && views.all {it.alpha==1f}};ready}
        val samples=synchronized(frames){frames.toList()}
        val values=samples.map {it[android.view.FrameMetrics.TOTAL_DURATION]/1_000_000.0}.sorted()
        assertTrue("The real timeline must deliver frames during scroll",values.size>30)
        val p95=values[(values.size*.95).toInt()]
        report("TIMELINE_SCROLL objects=$renderObjects frames=${values.size} median_ms=${values[values.size/2]} p95_ms=$p95 over_32ms=${values.count {it>32}} max_ms=${values.last()}")
        val labels=listOf("delay","input","animation","layout","draw","sync","command","swap","total")
        report("TIMELINE_PHASES objects=$renderObjects "+labels.mapIndexed {i,name->val phase=samples.map {it[i]/1_000_000.0}.sorted();"$name:p95=${phase[(phase.size*.95).toInt()]}"}.joinToString(" "))
        samples.sortedByDescending {it[8]}.take(5).forEach {sample->report("TIMELINE_WORST objects=$renderObjects "+labels.mapIndexed {i,name->"$name=${sample[i]/1_000_000.0}"}.joinToString(" "))}
        assertTrue("95% of real scroll frames should fit within two 60 Hz frames; p95=$p95 ms",p95<33.34)
    }
    @Test fun moving_objects_keep_delivering_frames_while_their_display_size_changes() {
        val objects=listOf(0 to 30,1 to 6,2 to 6)
        ui.runOnUiThread {ui.activity.setSigilContent {
            Row {objects.forEach {(kind,sides)->MaterialObject(kind,sides,1,null,"Sample",Modifier.weight(1f).height(132.dp))}}
        }}
        ui.waitUntil(15000) {var ready=false;ui.runOnUiThread {val views=textures(ui.activity.window.decorView);ready=views.size==3&&views.all {it.alpha==1f}};ready}
        val times=List(3){mutableListOf<Long>()}
        ui.runOnUiThread {
            val views=textures(ui.activity.window.decorView).map {it as MessageMaterialView}
            val sizes=views.map {it.width to it.height}
            views.forEachIndexed {i,v->val previous=v.surfaceTextureListener!!;v.surfaceTextureListener=object:TextureView.SurfaceTextureListener by previous {
                override fun onSurfaceTextureUpdated(s:android.graphics.SurfaceTexture) {times[i].add(System.nanoTime());previous.onSurfaceTextureUpdated(s)}
            }}
            val start=System.nanoTime()
            android.view.Choreographer.getInstance().postFrameCallback(object:android.view.Choreographer.FrameCallback {
                override fun doFrame(time:Long) {
                    val elapsed=(time-start)/1_000_000_000.0
                    val scale=1+.12*kotlin.math.sin(elapsed*5)
                    val q=floatArrayOf(kotlin.math.sin(elapsed).toFloat(),0f,0f,kotlin.math.cos(elapsed).toFloat())
                    views.forEachIndexed {i,v->
                        v.layout(v.left,v.top,v.left+(sizes[i].first*scale).toInt(),v.top+(sizes[i].second*scale).toInt())
                        v.update(MaterialFrame(objects[i].first,objects[i].second,1,0,0xff706080.toInt(),0,.5f,q,"Sample",true),true)
                    }
                    if(elapsed<4)android.view.Choreographer.getInstance().postFrameCallback(this)
                }
            })
        }
        android.os.SystemClock.sleep(4500)
        ui.runOnUiThread {times.forEachIndexed {i,t->
            assertTrue("Scaled object $i delivered only ${t.size} frames",t.size>80)
            val gaps=t.zipWithNext {a,b->(b-a)/1_000_000.0}.sorted()
            report("MATERIAL_SCALE object=$i frames=${t.size} median_ms=${gaps[gaps.size/2]} p95_ms=${gaps[(gaps.size*.95).toInt()]}")
        }}
    }
    @Test fun six_dice_frame_delivery() {
        val sides=listOf(6,10,12,16,24,30)
        val started=System.nanoTime()
        ui.runOnUiThread {ui.activity.setSigilContent {
            Column {sides.chunked(3).forEach {row->Row {row.forEach {n->MaterialObject(0,n,1,null,null,Modifier.weight(1f).height(132.dp))}}}}
        }}
        ui.waitUntil(15000) {var ready=false;ui.runOnUiThread {val views=textures(ui.activity.window.decorView);ready=views.size==6&&views.all {it.alpha==1f}};ready}
        report("MATERIAL_READY count=6 elapsed_ms=${(System.nanoTime()-started)/1_000_000}")
        val times=List(6){mutableListOf<Long>()}
        var views=emptyList<MessageMaterialView>()
        ui.runOnUiThread {views=textures(ui.activity.window.decorView).map {it as MessageMaterialView};views.forEachIndexed {i,v->val previous=v.surfaceTextureListener!!;v.surfaceTextureListener=object:TextureView.SurfaceTextureListener by previous {override fun onSurfaceTextureUpdated(s:android.graphics.SurfaceTexture) {times[i].add(System.nanoTime());previous.onSurfaceTextureUpdated(s)}}}}
        for(quality in listOf(1f,.5f)) {
            ui.runOnUiThread {
                times.forEach {it.clear()}
                val start=System.nanoTime()
                android.view.Choreographer.getInstance().postFrameCallback(object:android.view.Choreographer.FrameCallback {
                    override fun doFrame(time:Long) {
                        val elapsed=(time-start)/1_000_000_000.0
                        val angle=elapsed*3
                        val q=floatArrayOf(kotlin.math.sin(angle/2).toFloat(),0f,0f,kotlin.math.cos(angle/2).toFloat())
                        views.forEachIndexed {i,v->v.update(MaterialFrame(0,sides[i],1,0,0xff706080.toInt(),0,quality,q,null,true),true)}
                        if(elapsed<7)android.view.Choreographer.getInstance().postFrameCallback(this)
                    }
                })
            }
            android.os.SystemClock.sleep(7500)
            ui.runOnUiThread {times.forEachIndexed {i,t->assertTrue(t.size>2);val gaps=t.zipWithNext {a,b->(b-a)/1_000_000.0}.sorted();report("MATERIAL_PERF quality=$quality die=$i frames=${t.size} median_ms=${gaps[gaps.size/2]} p95_ms=${gaps[(gaps.size*.95).toInt()]}")}}
        }
    }
}
