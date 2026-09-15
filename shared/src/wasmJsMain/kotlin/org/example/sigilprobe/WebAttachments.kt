@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class,androidx.compose.ui.ExperimentalComposeUiApi::class)
package org.sigil

import androidx.compose.runtime.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.IntSize
import androidx.compose.foundation.clickable
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.WebElementView
import kotlinx.browser.document
import kotlinx.coroutines.*
import org.w3c.dom.*
import kotlin.js.*

internal val LocalWebFileSave=staticCompositionLocalOf<suspend(WebFile,JsAny)->Unit> {error("File saving unavailable")}
internal data class WebFile(val peer:String,val author:String,val message:String,val name:String,val type:String,val bytes:Long,val caption:String="",val draft:String="",val levels:List<Float> = emptyList(),val duration:Long=0)
internal fun ChatMessage.webFile()=attachment?.let {WebFile(peer,author,id,it.name,it.mediaType,it.bytes,it.caption)}

@Composable internal fun WebAttachment(file:WebFile,load:suspend(WebFile)->String,open:(WebFile)->Unit,modifier:Modifier=Modifier,expanded:Boolean=false,outgoing:Boolean?=null) {
    var url by remember(file) {mutableStateOf<String?>(null)}
    var issue by remember(file) {mutableStateOf<String?>(null)}
    var saving by remember(file){mutableStateOf(false)}
    var imageWidth by remember(file){mutableIntStateOf(0)}
    var imageHeight by remember(file){mutableIntStateOf(0)}
    val save=LocalWebFileSave.current
    var loading by remember(file) {mutableStateOf(false)}
    val scope=rememberCoroutineScope()
    val ink=if(file.type.startsWith("image/")) MaterialTheme.colorScheme.onBackground else when(outgoing){true->MaterialTheme.colorScheme.onPrimary;false->MaterialTheme.colorScheme.onSurfaceVariant;null->MaterialTheme.colorScheme.onSurface}
    val appearance=LocalAppearance.current
    val media=file.type in setOf("image/jpeg","image/png","image/webp","image/gif","audio/mpeg","audio/ogg","audio/webm","audio/mp4","audio/wav","video/mp4","video/webm","video/ogg")
    suspend fun fetch() {
        if(loading||url!=null)return
        loading=true;issue=null
        var created:String?=null
        try {created=load(file);currentCoroutineContext().ensureActive();url=created;created=null}
        catch(cancelled:CancellationException){throw cancelled}
        catch(_:Exception){issue="Could not load this attachment. Check your connection and retry."}
        finally {created?.let(::browserRevokeFileUrl);loading=false}
    }
    LaunchedEffect(file,expanded) {if(media && (expanded && file.bytes<=128*1024*1024 || file.bytes<=8*1024*1024) && (expanded || file.type!="image/gif" || appearance.autoplayGifs && !appearance.reducedMotion))fetch()}
    DisposableEffect(file) {onDispose {url?.let(::browserRevokeFileUrl)}}
    Column(modifier,verticalArrangement=Arrangement.spacedBy(8.dp)) {
        val current=url
        if(current!=null && media) {
            if(file.type.startsWith("audio/")) WebAudio(current,file.levels,file.draft.isNotEmpty(),file.duration)
            else if(file.type.startsWith("image/") && !expanded) ImageMessageFrame(imageWidth,imageHeight) { frame ->
                Box(frame) {
                    WebMedia(current,file.type,file.name,Modifier.fillMaxSize(),onDimensions={w,h->imageWidth=w;imageHeight=h},open=if(file.draft.isEmpty())({open(file)})else null)
                    if(file.type=="image/gif") GifChip(Modifier.align(Alignment.TopStart))
                }
            }
            else if(file.type.startsWith("video/") && !expanded) Box(Modifier.fillMaxWidth().height(200.dp),contentAlignment=Alignment.Center) {
                WebMedia(current,file.type,file.name,Modifier.fillMaxSize(),open={open(file)})
                Surface(shape=androidx.compose.foundation.shape.CircleShape,color=androidx.compose.ui.graphics.Color.Black.copy(alpha=.6f),contentColor=androidx.compose.ui.graphics.Color.White) { SigilIconButton({open(file)}) { Glyph("play_arrow",28,"Play video") } }
            }
            else MediaViewerFrame(imageWidth,imageHeight,Modifier.weight(1f)) { frame -> WebMedia(current,file.type,file.name,frame,interactive=expanded,onDimensions={w,h->imageWidth=w;imageHeight=h}) }

        }
        else if(file.type.startsWith("image/") || file.type.startsWith("video/")) Box(Modifier.fillMaxWidth().height(200.dp).background(MaterialTheme.colorScheme.surfaceContainerHigh),contentAlignment=Alignment.Center) {
            SigilIconButton({if(!expanded && file.draft.isEmpty())open(file) else scope.launch {fetch()}},enabled=(!expanded && file.draft.isEmpty()) || !loading && file.bytes<=128*1024*1024) {Glyph(if(issue!=null)"refresh" else if(file.type.startsWith("video/"))"play_arrow" else "download",28,if(issue!=null)"Retry attachment" else if(!expanded && file.draft.isEmpty())"Open attachment" else "Load attachment")}
            if(file.type=="image/gif")GifChip(Modifier.align(Alignment.TopStart))
        }
        else Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
            Glyph(if(media)"play_circle" else "description",32)
            Column(Modifier.weight(1f)) {Text(file.name,style=MaterialTheme.typography.titleSmall);Text("${file.bytes/1024} KiB",style=MaterialTheme.typography.labelSmall)}
        }
        if(loading)LinearProgressIndicator(Modifier.fillMaxWidth())
        issue?.let {Text(it,style=MaterialTheme.typography.bodySmall)}
        if(!file.type.startsWith("image/") && !file.type.startsWith("video/") && (current==null || !expanded)) Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            if(current==null)SigilTextButton({scope.launch {fetch()}},enabled=!loading && file.bytes<=128*1024*1024){Text(if(issue!=null)"Retry" else "Load attachment",color=ink)}
            if(!expanded && file.draft.isEmpty())SigilIconButton({open(file)}){Glyph("open_in_full",24,"Expand attachment")}
            if(file.draft.isEmpty() && browserFileStreamSupported())SigilIconButton({
                val destination=browserFileDestination(file.name)
                scope.launch {saving=true;try{destination.awaitBrowser<JsAny?>()?.let {save(file,it)}}catch(cancelled:CancellationException){throw cancelled}catch(_:Exception){issue="Could not save this attachment. Check your connection and available space."}finally{saving=false}}
            },enabled=!saving){Glyph("download",24,if(saving)"Saving attachment" else "Save attachment")}
            else if(current!=null && file.draft.isEmpty())SigilIconButton({browserSaveFileUrl(current,file.name)}){Glyph("download",24,"Save attachment")}
        }
        if(expanded && !file.type.startsWith("image/") && !file.type.startsWith("video/") && file.caption.isNotBlank())Text(file.caption)
        if(file.bytes>128*1024*1024)Text(if(browserFileStreamSupported())"Save this file to open it. Inline viewing supports up to 128 MiB." else "This browser supports saving up to 128 MiB. Use a browser with file-system access or the Android app for larger files.",style=MaterialTheme.typography.bodySmall)
    }
}

@Composable private fun WebAudio(url:String,levels:List<Float>,preview:Boolean,recordedDuration:Long) {
    val audio=remember(url){(document.createElement("audio") as HTMLAudioElement).apply {src=url;preload="metadata"}}
    var playing by remember(url){mutableStateOf(false)}
    var position by remember(url){mutableLongStateOf(0)}
    var duration by remember(url){mutableLongStateOf(recordedDuration)}
    var issue by remember(url){mutableStateOf(false)}
    var ready by remember(url){mutableStateOf(false)}
    val scope=rememberCoroutineScope()
    LaunchedEffect(audio){
        duration=resolveWebAudioDuration(audio).takeIf {it>0} ?: duration;ready=true
        while(isActive){position=(audio.currentTime*1000).toLong().coerceAtLeast(0);duration=audio.duration.takeIf {it.isFinite() && it>0}?.let {(it*1000).toLong()} ?: duration;playing=!audio.paused;delay(if(playing)80 else 250)}
    }
    DisposableEffect(audio){onDispose {audio.pause();audio.removeAttribute("src");audio.load()}}
    AudioPlayback(position,duration,playing,levels,enabled=ready,preview=preview,modifier=Modifier.fillMaxWidth().padding(horizontal=12.dp),
        play={if(playing)audio.pause() else scope.launch {try {audio.play().await<JsAny?>();issue=false}catch(_:Exception){issue=true}}},
        seek={audio.currentTime=it/1000.0})
    if(issue)Text("Could not play audio. Try again.",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.error)
}

internal suspend fun resolveWebAudioDuration(audio:HTMLAudioElement):Long {
    try {
        withTimeoutOrNull(5000){
            while(audio.readyState<1 && audio.error==null)delay(20)
            if(audio.duration==Double.POSITIVE_INFINITY && audio.src.startsWith("blob:")){
                // Chunked MediaRecorder WebM omits duration; seeking discovers the local file's end.
                audio.currentTime=1e9
                while(!audio.duration.isFinite() && audio.error==null)delay(20)
            }
        }
        return audio.duration.takeIf {it.isFinite() && it>0}?.let {(it*1000).toLong()} ?: 0
    }catch(cancelled:CancellationException){throw cancelled}
    catch(_:Exception){return 0}
    finally {runCatching {audio.currentTime=0.0}}
}

@Composable private fun WebMedia(url:String,type:String,name:String,modifier:Modifier,interactive:Boolean=false,onDimensions:(Int,Int)->Unit={_,_->},open:(()->Unit)?=null) {
    var zoom by remember(url) { mutableFloatStateOf(1f) }
    var pan by remember(url) { mutableStateOf(Offset.Zero) }
    var size by remember { mutableStateOf(IntSize.Zero) }
    fun bounded(value:Offset,z:Float)=Offset(value.x.coerceIn(-size.width*(z-1)/2,size.width*(z-1)/2),value.y.coerceIn(-size.height*(z-1)/2,size.height*(z-1)/2))
    val imageGestures=interactive && type.startsWith("image/") && type!="image/gif"
    val timeline=LocalMaterialTimeline.current.takeIf {open!=null && !LocalObjectMenu.current}
    val occlusion=LocalMaterialOcclusion.current
    val density=LocalDensity.current.density
    val visible=LocalMotionVisible.current
    val dimensions by rememberUpdatedState(onDimensions)
    val onOpen by rememberUpdatedState(open)
    val reduced=LocalMotion.current.reduced
    val stillLoader=remember(url,reduced) { mutableListOf<HTMLImageElement>() }
    val element=remember(url,reduced) {
        val tag=when {type.startsWith("image/")->"img";type.startsWith("audio/")->"audio";else->"video"}
        if(type=="image/gif" && reduced) {
            (document.createElement("canvas") as HTMLCanvasElement).apply {
                setAttribute("aria-label",name)
                setAttribute("style","display:block;width:100%;height:100%;object-fit:contain;border-radius:16px")
                val canvas=this
                val image=document.createElement("img") as HTMLImageElement
                stillLoader.add(image)
                image.onload={
                    val scale=minOf(1.0,1080.0/maxOf(image.naturalWidth,image.naturalHeight).coerceAtLeast(1))
                    canvas.width=(image.naturalWidth*scale).toInt().coerceAtLeast(1);canvas.height=(image.naturalHeight*scale).toInt().coerceAtLeast(1)
                    (canvas.getContext("2d") as CanvasRenderingContext2D).drawImage(image,0.0,0.0,canvas.width.toDouble(),canvas.height.toDouble())
                    dimensions(canvas.width,canvas.height);Unit
                }
                image.src=url
            }
        } else (document.createElement(tag) as HTMLElement).apply {
            setAttribute("src",url);setAttribute("aria-label",name)
            setAttribute("style","display:block;width:100%;height:100%;object-fit:contain;border-radius:16px")
            if(tag=="img") {
                setAttribute("alt",name)
                (this as HTMLImageElement).onload={dimensions(naturalWidth,naturalHeight);Unit}
            }
            else {
                if(open==null)setAttribute("controls","")
                setAttribute("preload","metadata");setAttribute("playsinline","")
                (this as? HTMLVideoElement)?.let { video -> video.onloadedmetadata={dimensions(video.videoWidth,video.videoHeight);Unit} }
            }
        }
    }
    val viewport=remember(element){(document.createElement("div") as HTMLElement).apply {
        setAttribute("style","width:100%;height:100%;overflow:auto;overscroll-behavior:contain;border-radius:16px")
        setAttribute("tabindex","0");setAttribute("aria-label","Attachment viewer");appendChild(element)
    }}
    LaunchedEffect(viewport,timeline,occlusion,density,visible) {
        if(timeline==null){viewport.style.removeProperty("clip-path");return@LaunchedEffect}
        viewport.style.setProperty("clip-path","inset(100%)")
        var previous=""
        while(isActive && visible) {
            withFrameNanos { }
            webInteropPointerPassThrough(viewport)
            val bounds=occlusion?.visible(timeline.viewport) ?: timeline.viewport
            val clip=Rect(bounds.left/density,bounds.top/density,bounds.right/density,bounds.bottom/density)
            val element=viewport.getBoundingClientRect()
            val notice=occlusion?.notice?.takeIf {it.width>0 && it.height>0}?.let {Rect(it.left/density,it.top/density,it.right/density,it.bottom/density)}
            val value=if(notice!=null)materialClipPath(clip,null,element.left.toFloat(),element.top.toFloat(),notice) else "inset("+materialClipInsets(clip,element.left.toFloat(),element.top.toFloat(),element.width.toFloat(),element.height.toFloat()).joinToString(" "){"${it}%"}+")"
            if(value!=previous){viewport.style.setProperty("clip-path",value);previous=value}
        }
    }
    val interaction=if(imageGestures) Modifier.semantics {contentDescription=name;customActions=listOf(CustomAccessibilityAction("Zoom in"){zoom=(zoom*2).coerceAtMost(8f);true},CustomAccessibilityAction("Zoom out"){zoom=(zoom/2).coerceAtLeast(1f);pan=bounded(pan,zoom);true},CustomAccessibilityAction("Reset image"){zoom=1f;pan=Offset.Zero;true})}.onSizeChanged {size=it}.pointerInput(url,size) { detectTransformGestures {centroid,movement,change,_->val next=(zoom*change).coerceIn(1f,8f);val focus=centroid-Offset(size.width/2f,size.height/2f);pan=bounded(focus-(focus-pan)*(next/zoom)+movement,next);zoom=next} }.pointerInput(url) {detectTapGestures(onDoubleTap={zoom=if(zoom>1f)1f else 2f;pan=Offset.Zero})} else if(open!=null) Modifier.clickable(role=Role.Button){onOpen?.invoke()}.semantics {contentDescription=name} else Modifier
    WebElementView(factory={viewport},modifier=modifier.then(interaction),update={
        viewport.style.setProperty("pointer-events",if(open!=null || imageGestures)"none" else "auto")
        viewport.setAttribute("tabindex",if(open!=null)"-1" else "0")
        if(imageGestures) {
            viewport.style.setProperty("overflow","hidden")
            element.style.setProperty("transform","translate(${pan.x/density}px,${pan.y/density}px) scale($zoom)")
        }

    },onRelease={
        stillLoader.forEach {it.onload=null;it.removeAttribute("src")}
        (element as? HTMLImageElement)?.onload=null
        (element as? HTMLVideoElement)?.onloadedmetadata=null
        (element as? HTMLMediaElement)?.pause();element.removeAttribute("src");(element as? HTMLMediaElement)?.load()
    })
}

@Composable internal fun WebFileViewer(file:WebFile,load:suspend(WebFile)->String,close:()->Unit) {
    val onClose by rememberUpdatedState(close)
    DisposableEffect(Unit){
        val listener:(org.w3c.dom.events.Event)->Unit={event->if((event as? org.w3c.dom.events.KeyboardEvent)?.key=="Escape"){event.preventDefault();onClose()}}
        document.addEventListener("keydown",listener)
        onDispose{document.removeEventListener("keydown",listener)}
    }
    val message=LocalMediaMessage.current(file.peer,file.author,file.message)
    val save=LocalWebFileSave.current
    val scope=rememberCoroutineScope()
    var issue by remember {mutableStateOf<String?>(null)}
    var saving by remember {mutableStateOf(false)}
    MediaViewerChrome(message,close,saveEnabled=!saving,save=({
        val destination=if(browserFileStreamSupported())browserFileDestination(file.name)else null
        scope.launch {
            saving=true
            try {
                if(destination!=null)destination.awaitBrowser<JsAny?>()?.let {save(file,it)}
                else {val url=load(file);try {browserSaveFileUrl(url,file.name)}finally {browserRevokeFileUrl(url)}}
            } catch(cancelled:CancellationException){throw cancelled}
            catch(_:Exception){issue="Could not save this attachment."}
            finally {saving=false}
        }
    })) {
        WebAttachment(file,load,{},Modifier.widthIn(max=1000.dp).fillMaxSize(),expanded=true)
    }
    issue?.let { text -> AlertDialog({issue=null},text={Text(text)},confirmButton={SigilTextButton({issue=null}){Text("OK")}}) }
}
