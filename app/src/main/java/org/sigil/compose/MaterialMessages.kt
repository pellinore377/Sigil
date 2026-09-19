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
    fun snapshotKey()=listOf(kind,sides,face,font,accent,backdrop,progress,rotation?.joinToString(","),label,transparent,style?.joinToString(",")).joinToString("") {if(it==null)"-1:" else it.toString().let {value->"${value.length}:$value"}}
    fun sameImage(other:MaterialFrame):Boolean = kind==other.kind && sides==other.sides && face==other.face && font==other.font && accent==other.accent && backdrop==other.backdrop && label==other.label && transparent==other.transparent && rotation.contentEquals(other.rotation) && style.contentEquals(other.style) && (if(rotation==null)progress==other.progress else (progress<1f)==(other.progress<1f))
}
private val materialUi = android.os.Handler(android.os.Looper.getMainLooper())
private val materialWorker = Executors.newSingleThreadExecutor { task -> Thread(task,"Sigil materials").apply { isDaemon=true } }
internal class MessageMaterialView(context:android.content.Context, private val limit:Int, private val captured:((MaterialFrame,android.graphics.Bitmap)->Unit)?=null, private val mirror:((android.graphics.Bitmap)->Unit)?=null, private val failed:()->Unit):TextureView(context),TextureView.SurfaceTextureListener {
    // Each frame is read back into one of two bitmaps for Compose to draw, so the object lives in the page beneath the glass.
    private val mirrors=arrayOfNulls<android.graphics.Bitmap>(2)
    private var mirrorIndex=0
    private val latest=AtomicReference<MaterialFrame?>(null)
    private val queued=AtomicBoolean(false)
    @Volatile private var active=true
    @Volatile private var generation=0
    private var nativeId=0L
    private var bufferWidth=0
    private var bufferHeight=0
    private var first:MaterialFrame?=null
    private var pristine=true
    private var snapshotTaken=false
    private var shown=false
    init { isOpaque=false;alpha=0f;surfaceTextureListener=this }
    fun update(frame:MaterialFrame,visible:Boolean) {
        if(first==null)first=frame else if(first?.sameImage(frame)!=true)pristine=false
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
                if(result==1)post {if(token==generation && !shown) {shown=true;if(mirror==null)animate().alpha(1f).setDuration(160).start()}}
                if(result==0)post {if(token==generation)failed()}
            }
            queued.set(false)
            if(active && token==generation && result!=0 && (latest.get()!=frame || result==2))postOnAnimation {schedule()}
        }
    }
    override fun onSurfaceTextureAvailable(texture:SurfaceTexture,width:Int,height:Int) {
        val scale=limit.toFloat()/maxOf(width,height)
        val w=(width*scale).toInt().coerceAtLeast(1);val h=(height*scale).toInt().coerceAtLeast(1)
        bufferWidth=w;bufferHeight=h
        texture.setDefaultBufferSize(w,h)
        val surface=Surface(texture);val token=++generation
        fun create() {
            materialWorker.execute {
                if(token!=generation) {surface.release();return@execute}
                if(nativeId>0L)MaterialNative.destroy(nativeId)
                val created=MaterialNative.create(surface,w,h)
                nativeId=created.coerceAtLeast(0L)
                if(created==-1L) {
                    // Lazy rows and the flight overlay briefly overlap. Keep the
                    // surface alive and retry once departing rows release capacity.
                    materialUi.postDelayed({create()},100)
                } else {
                    surface.release()
                    if(created==0L)post {if(token==generation)failed()}
                    else schedule()
                }
            }
        }
        create()
    }
    override fun onSurfaceTextureSizeChanged(texture:SurfaceTexture,width:Int,height:Int) {
        if(bufferWidth==0 || kotlin.math.abs(width.toFloat()/height-bufferWidth.toFloat()/bufferHeight)>.02f)
            onSurfaceTextureAvailable(texture,width,height)
    }
    override fun onSurfaceTextureUpdated(texture:SurfaceTexture) {
        mirror?.let {send->
            if(bufferWidth>0 && bufferHeight>0) {
                val target=mirrors[mirrorIndex]?.takeIf {it.width==bufferWidth && it.height==bufferHeight} ?: android.graphics.Bitmap.createBitmap(bufferWidth,bufferHeight,android.graphics.Bitmap.Config.ARGB_8888).also {mirrors[mirrorIndex]=it}
                mirrorIndex=(mirrorIndex+1)%2
                if(getBitmap(target)!=null)send(target)
            }
        }
        val capture=captured ?: return
        val frame=first ?: return
        // An unchanged, initially settled view has only ever submitted this exact
        // image. Never read back an in-flight orientation as its final result.
        if(!active || snapshotTaken || !pristine || frame.progress<1f || latest.get()?.sameImage(frame)!=true)return
        val image=getBitmap(bufferWidth,bufferHeight) ?: return
        snapshotTaken=true
        capture(frame,image)
    }
    override fun onSurfaceTextureDestroyed(texture:SurfaceTexture):Boolean {
        generation++
        materialWorker.execute {if(nativeId!=0L){MaterialNative.destroy(nativeId);nativeId=0L};texture.release()}
        return false
    }
    fun close() {active=false;generation++;materialWorker.execute {if(nativeId!=0L){MaterialNative.destroy(nativeId);nativeId=0L}}}
}
