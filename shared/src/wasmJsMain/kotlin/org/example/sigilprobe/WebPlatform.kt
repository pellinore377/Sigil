@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class,kotlin.io.encoding.ExperimentalEncodingApi::class)
package org.sigil

import androidx.compose.foundation.Image
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.layout.ContentScale
import kotlinx.browser.document
import kotlinx.browser.window
import kotlinx.coroutines.*
import org.w3c.dom.*
import kotlin.io.encoding.Base64
import kotlin.js.*

private external interface ScreenWakeLock:JsAny {fun release():Promise<JsAny?>}
private external interface WakeLockApi:JsAny {fun request(type:String):Promise<ScreenWakeLock>}
private external interface WakeNavigator:JsAny {val wakeLock:WakeLockApi?}

/** Follows the system's reduced-motion preference as Android follows its animator scale. */
@Composable internal fun rememberWebReducedMotion():Boolean {
    val query=remember {runCatching {window.matchMedia("(prefers-reduced-motion: reduce)")}.getOrNull()}
    var reduced by remember {mutableStateOf(query?.matches==true)}
    DisposableEffect(query) {
        val listener:(org.w3c.dom.events.Event)->Unit={reduced=query?.matches==true}
        query?.addEventListener("change",listener)
        onDispose {query?.removeEventListener("change",listener)}
    }
    return reduced
}

/** Native form controls, scrollbars and media controls follow the app's light or dark mode. */
internal fun webColorScheme(dark:Boolean) {(document.documentElement as? HTMLElement)?.style?.setProperty("color-scheme",if(dark)"dark" else "light")}

/** Screen wake lock; the browser drops it when the page hides, so it is retaken on return. */
@Composable internal fun WebKeepAwake(enabled:Boolean) {
    val visible=LocalMotionVisible.current
    LaunchedEffect(enabled,visible) {
        if(!enabled || !visible)return@LaunchedEffect
        val api=window.navigator.unsafeCast<WakeNavigator>().wakeLock ?: return@LaunchedEffect
        val lock=try {api.request("screen").awaitBrowser()}catch(cancelled:CancellationException){throw cancelled}catch(_:Exception){return@LaunchedEffect}
        try {awaitCancellation()} finally {runCatching {lock.release()}}
    }
}

private const val ThumbnailSide=192
private const val ThumbnailLimit=8L*1024*1024
private val thumbnails=LinkedHashMap<String,ImageBitmap>()

/** A quoted picture or clip, cropped square from the browser's own decode. */
@Composable internal fun WebThumbnail(message:ChatMessage,load:suspend(WebFile)->String,modifier:Modifier):Boolean {
    val file=message.webFile() ?: return false
    val video=file.type.startsWith("video/")
    if(!video && !file.type.startsWith("image/") || file.bytes>ThumbnailLimit)return false
    val key="${file.peer}/${file.author}/${file.message}"
    var image by remember(key) {mutableStateOf(thumbnails[key])}
    var failed by remember(key) {mutableStateOf(false)}
    LaunchedEffect(key) {
        if(image==null)image=try {webPoster(load(file),video)?.also {thumbnails[key]=it;while(thumbnails.size>48)thumbnails.remove(thumbnails.keys.first())}}catch(cancelled:CancellationException){throw cancelled}catch(_:Exception){null}
        // An undecodable clip or picture falls back to the file glyph.
        failed=image==null
    }
    image?.let {Image(it,null,modifier,contentScale=ContentScale.Crop)}
    return !failed
}

private suspend fun webPoster(url:String,video:Boolean):ImageBitmap? {
    val clip=if(video)(document.createElement("video") as HTMLVideoElement).apply {muted=true;preload="auto";setAttribute("playsinline","");src=url} else null
    val picture=if(video)null else (document.createElement("img") as HTMLImageElement).apply {src=url}
    try {
        val ready=withTimeoutOrNull(10000) {
            if(clip!=null) {
                while(clip.readyState<1 && clip.error==null)delay(20)
                if(clip.error!=null)return@withTimeoutOrNull false
                clip.currentTime=clip.duration.takeIf {it.isFinite() && it>0}?.let {minOf(.1,it/2)} ?: 0.0
                while((clip.readyState<2 || clip.seeking) && clip.error==null)delay(20)
                clip.error==null
            } else {
                while(!picture!!.complete)delay(20)
                picture.naturalWidth>0
            }
        } ?: false
        if(!ready)return null
        val width=clip?.videoWidth ?: picture!!.naturalWidth
        val height=clip?.videoHeight ?: picture!!.naturalHeight
        if(width<=0 || height<=0)return null
        val side=minOf(width,height).toDouble()
        val canvas=(document.createElement("canvas") as HTMLCanvasElement).apply {this.width=ThumbnailSide;this.height=ThumbnailSide}
        val context=canvas.getContext("2d") as CanvasRenderingContext2D
        val source:CanvasImageSource=clip ?: picture!!
        context.drawImage(source,(width-side)/2,(height-side)/2,side,side,0.0,0.0,ThumbnailSide.toDouble(),ThumbnailSide.toDouble())
        val bytes=Base64.decode(canvas.toDataURL("image/png").substringAfter(','))
        return org.jetbrains.skia.Image.makeFromEncoded(bytes).toComposeImageBitmap()
    } finally {
        clip?.let {it.removeAttribute("src");it.load()}
        picture?.removeAttribute("src")
    }
}
