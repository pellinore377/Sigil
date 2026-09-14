package org.sigil.compose

import android.graphics.SurfaceTexture
import android.view.Surface
import android.view.TextureView
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

internal object MaterialNative {
    val available = runCatching { System.loadLibrary("sigil_material_android") }.isSuccess
    @JvmStatic external fun create(surface: Surface, width: Int, height: Int): Long
    @JvmStatic external fun draw(id: Long, kind: Int, sides: Int, face: Int, font: Int, accent: Int, backdrop: Int, progress: Float, rotation: FloatArray?, label: String?, transparent: Boolean, style:FloatArray?): Int
    @JvmStatic external fun horizontalExtent(sides:Int,face:Int,rotation:FloatArray?,outgoing:Boolean):Float
    @JvmStatic external fun record(data:FloatArray):FloatArray?
    @JvmStatic external fun destroy(id: Long)
}
internal data class MaterialFrame(val kind:Int,val sides:Int,val face:Int,val font:Int,val accent:Int,val backdrop:Int,val progress:Float,val rotation:FloatArray?=null,val label:String?=null,val transparent:Boolean=false,val style:FloatArray?=null) {
    fun sameImage(other:MaterialFrame):Boolean = kind==other.kind && sides==other.sides && face==other.face && font==other.font && accent==other.accent && backdrop==other.backdrop && label==other.label && transparent==other.transparent && rotation.contentEquals(other.rotation) && style.contentEquals(other.style) && (if(rotation==null)progress==other.progress else (progress<1f)==(other.progress<1f))
}
private val materialWorker = Executors.newSingleThreadExecutor { task -> Thread(task,"Sigil materials").apply { isDaemon=true } }
internal class MessageMaterialView(context:android.content.Context, private val limit:Int, private val failed:()->Unit):TextureView(context),TextureView.SurfaceTextureListener {
    private val latest=AtomicReference<MaterialFrame?>(null)
    private val queued=AtomicBoolean(false)
    @Volatile private var active=true
    @Volatile private var generation=0
    private var nativeId=0L
    init { isOpaque=false;alpha=0f;surfaceTextureListener=this }
    fun update(frame:MaterialFrame,visible:Boolean) {
        val changed=latest.get()?.sameImage(frame)!=true
        val resumed=visible&&!active
        if(changed)latest.set(frame)
        active=visible
        if(visible&&(changed||resumed))schedule()
    }
    private fun schedule() {
        if(!MaterialNative.available || !active || !queued.compareAndSet(false,true))return
        val token=generation
        materialWorker.execute {
            val frame=latest.get()
            var result=1
            if(active && token==generation && nativeId!=0L && frame!=null) {
                result=MaterialNative.draw(nativeId,frame.kind,frame.sides,frame.face,frame.font,frame.accent,frame.backdrop,frame.progress,frame.rotation,frame.label,frame.transparent,frame.style)
                if(result==1)post {if(token==generation)alpha=1f}
                if(result==0)post {if(token==generation)failed()}
            }
            queued.set(false)
            if(active && token==generation && result!=0 && (latest.get()!=frame || result==2))postOnAnimation {schedule()}
        }
    }
    override fun onSurfaceTextureAvailable(texture:SurfaceTexture,width:Int,height:Int) {
        val scale=minOf(1f,limit.toFloat()/maxOf(width,height))
        val w=(width*scale).toInt().coerceAtLeast(1);val h=(height*scale).toInt().coerceAtLeast(1)
        texture.setDefaultBufferSize(w,h)
        val surface=Surface(texture);val token=++generation
        materialWorker.execute {
            if(nativeId!=0L)MaterialNative.destroy(nativeId)
            nativeId=if(token==generation)MaterialNative.create(surface,w,h)else 0L
            surface.release()
            if(nativeId==0L && token==generation)post {failed()}
            schedule()
        }
    }
    override fun onSurfaceTextureSizeChanged(texture:SurfaceTexture,width:Int,height:Int) {
        onSurfaceTextureAvailable(texture,width,height)
    }
    override fun onSurfaceTextureUpdated(texture:SurfaceTexture) {}
    override fun onSurfaceTextureDestroyed(texture:SurfaceTexture):Boolean {
        generation++
        materialWorker.execute {if(nativeId!=0L){MaterialNative.destroy(nativeId);nativeId=0L};texture.release()}
        return false
    }
    fun close() {active=false;generation++;materialWorker.execute {if(nativeId!=0L){MaterialNative.destroy(nativeId);nativeId=0L}}}
}
