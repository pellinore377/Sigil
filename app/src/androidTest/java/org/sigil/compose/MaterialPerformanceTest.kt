package org.sigil.compose

import android.view.TextureView
import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class MaterialPerformanceTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    private fun report(value:String) {androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().sendStatus(0,android.os.Bundle().apply {putString("stream","\n$value\n")})}
    private fun textures(v:View):List<TextureView> = when(v) {is TextureView->listOf(v);is ViewGroup->(0 until v.childCount).flatMap {textures(v.getChildAt(it))};else->emptyList()}
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
