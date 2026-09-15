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

internal class ImageCache(private val budget: Int = 24 * 1024 * 1024) {
    private val entries=LinkedHashMap<String,Bitmap>(16,.75f,true)
    private val loading=Mutex()
    private var bytes=0
    private var generation=0L
    private var active=true
    @Synchronized private fun cached(key:String)=entries[key]
    @Synchronized fun clear(){entries.clear();bytes=0;generation++}
    @Synchronized fun setActive(value:Boolean){active=value;if(!value)clear()}
    @Synchronized internal fun retainedBytes()=bytes
    suspend fun load(key:String,decode:suspend()->Bitmap):Bitmap {
        cached(key)?.let {return it}
        return loading.withLock {
            cached(key)?.let {return@withLock it}
            val epoch=synchronized(this){generation}
            val bitmap=decode()
            synchronized(this) {
                if(active && epoch==generation && bitmap.allocationByteCount<=budget) {
                    entries.put(key,bitmap)?.let {bytes-=it.allocationByteCount}
                    bytes+=bitmap.allocationByteCount
                    val iterator=entries.entries.iterator()
                    while((bytes>budget || entries.size>64) && iterator.hasNext()){bytes-=iterator.next().value.allocationByteCount;iterator.remove()}
                }
            }
            bitmap
        }
    }
}
internal val LocalImageCache=staticCompositionLocalOf<ImageCache?> {null}

@Composable internal fun rememberImageCache(state:MessengerState):ImageCache {
    val owner=if(state.phase=="connected")listOf(state.address,state.fingerprint,state.device) else emptyList()
    val cache=remember(owner){ImageCache()}
    val lifecycle=LocalLifecycleOwner.current.lifecycle
    val context=LocalContext.current.applicationContext
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
