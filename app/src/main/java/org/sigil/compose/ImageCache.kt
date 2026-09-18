package org.sigil.compose

import android.content.ComponentCallbacks2
import android.content.res.Configuration
import android.graphics.Bitmap
import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.sigil.MessengerState

private val materialWriter = java.util.concurrent.Executors.newSingleThreadExecutor()
private const val MaterialFiles = 512

internal class ImageCache(private val budget: Int = 24 * 1024 * 1024, private val store: java.io.File? = null) {
    private val entries=LinkedHashMap<String,Bitmap>(16,.75f,true)
    private val shapes=LinkedHashMap<String,Pair<Int,Int>>(16,.75f,true)
    private val loading=Mutex()
    private var bytes=0
    private var generation=0L
    private var active=true
    @Synchronized private fun cached(key:String)=entries[key]
    @Synchronized fun clear(){entries.clear();shapes.clear();bytes=0;generation++}
    /// A picture already decoded knows its own size; no header has to be read for it.
    @Synchronized fun retained(key:String)=if(active)entries[key] else null
    /// Picture dimensions outlive the decoded bitmap so a frame is the right size before its content arrives.
    @Synchronized fun shape(key:String)=if(active)shapes[key] else null
    @Synchronized fun rememberShape(key:String,width:Int,height:Int) {
        if(!active || width<=0 || height<=0)return
        shapes[key]=width to height
        while(shapes.size>1024)shapes.remove(shapes.keys.first())
    }
    @Synchronized fun setActive(value:Boolean){active=value;if(!value)clear()}
    @Synchronized internal fun retainedBytes()=bytes
    @Synchronized internal fun retainedMaterials()=entries.keys.count {it.startsWith("material:")}
    @Synchronized internal fun materialGeneration()=generation
    @Synchronized internal fun materialSnapshot(key:String)=if(active)entries["material:$key"] else null
    @Synchronized internal fun rememberMaterial(key:String,bitmap:Bitmap,epoch:Long) {
        retain("material:$key",bitmap,epoch)
        val file=materialFile(key)?.takeIf {active && !it.isFile} ?: return
        materialWriter.execute {runCatching {
            file.parentFile?.mkdirs()
            file.outputStream().use {bitmap.compress(Bitmap.CompressFormat.PNG,100,it)}
            val files=file.parentFile?.listFiles().orEmpty()
            if(files.size>MaterialFiles)files.sortedBy {it.lastModified()}.take(files.size-MaterialFiles).forEach {it.delete()}
        }}
    }
    /// A settled object drawn once is kept on disk, so a later launch shows it without a GPU surface.
    suspend fun loadMaterial(key:String):Bitmap? {
        materialSnapshot(key)?.let {return it}
        val file=materialFile(key)?.takeIf {it.isFile} ?: return null
        val bitmap=kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {android.graphics.BitmapFactory.decodeFile(file.path)} ?: return null
        synchronized(this) {retain("material:$key",bitmap,generation)}
        return bitmap
    }
    private fun materialFile(key:String)=store?.let {dir->
        val digest=java.security.MessageDigest.getInstance("SHA-256").digest(key.toByteArray())
        java.io.File(dir,"m-"+digest.take(16).joinToString("") {"%02x".format(it)}+".png")
    }
    private fun retain(key:String,bitmap:Bitmap,epoch:Long) {
        if(active && epoch==generation && bitmap.allocationByteCount<=budget) {
            entries.put(key,bitmap)?.let {bytes-=it.allocationByteCount}
            bytes+=bitmap.allocationByteCount
            val iterator=entries.entries.iterator()
            while((bytes>budget || entries.size>64) && iterator.hasNext()){bytes-=iterator.next().value.allocationByteCount;iterator.remove()}
        }
    }
    suspend fun load(key:String,decode:suspend()->Bitmap):Bitmap {
        cached(key)?.let {return it}
        return loading.withLock {
            cached(key)?.let {return@withLock it}
            val epoch=synchronized(this){generation}
            val bitmap=decode()
            synchronized(this) {
                retain(key,bitmap,epoch)
            }
            bitmap
        }
    }
}
internal val LocalImageCache=staticCompositionLocalOf<ImageCache?> {null}

@Composable internal fun rememberImageCache(state:MessengerState):ImageCache {
    val owner=if(state.phase=="connected")listOf(state.address,state.fingerprint,state.device) else emptyList()
    val context=LocalContext.current.applicationContext
    val cache=remember(owner){ImageCache(store=java.io.File(context.cacheDir,"materials"))}
    val lifecycle=LocalLifecycleOwner.current.lifecycle
    DisposableEffect(cache,lifecycle,context) {
        cache.setActive(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED))
        val observer=LifecycleEventObserver {_,event->when(event){Lifecycle.Event.ON_STOP->cache.setActive(false);Lifecycle.Event.ON_START->cache.setActive(true);else->Unit}}
        val memory=object:ComponentCallbacks2 {
            override fun onConfigurationChanged(value:Configuration){}
            override fun onLowMemory(){cache.clear()}
            override fun onTrimMemory(level:Int){if(level>=ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW)cache.clear()}
        }
        lifecycle.addObserver(observer);context.registerComponentCallbacks(memory)
        onDispose {lifecycle.removeObserver(observer);context.unregisterComponentCallbacks(memory);cache.setActive(false)}
    }
    return cache
}
