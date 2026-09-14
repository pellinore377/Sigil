@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class,androidx.compose.ui.ExperimentalComposeUiApi::class)
package org.sigil

import androidx.compose.runtime.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.background
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.WebElementView
import kotlinx.browser.document
import kotlinx.coroutines.*
import org.w3c.dom.*
import kotlin.js.*

internal val LocalWebFileSave=staticCompositionLocalOf<suspend(WebFile,JsAny)->Unit> {error("File saving unavailable")}
internal data class WebFile(val peer:String,val author:String,val message:String,val name:String,val type:String,val bytes:Long,val caption:String="",val draft:String="")
internal fun ChatMessage.webFile()=attachment?.let {WebFile(peer,author,id,it.name,it.mediaType,it.bytes,it.caption)}

@Composable internal fun WebAttachment(file:WebFile,load:suspend(WebFile)->String,open:(WebFile)->Unit,modifier:Modifier=Modifier,expanded:Boolean=false,outgoing:Boolean?=null) {
    var url by remember(file) {mutableStateOf<String?>(null)}
    var issue by remember(file) {mutableStateOf<String?>(null)}
    var saving by remember(file){mutableStateOf(false)}
    var zoom by remember(file){mutableIntStateOf(1)}
    val save=LocalWebFileSave.current
    var loading by remember(file) {mutableStateOf(false)}
    val scope=rememberCoroutineScope()
    val ink=when(outgoing){true->MaterialTheme.colorScheme.onPrimary;false->MaterialTheme.colorScheme.onSurfaceVariant;null->MaterialTheme.colorScheme.onSurface}
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
    LaunchedEffect(file) {if(media && file.bytes<=8*1024*1024 && (file.type!="image/gif" || appearance.autoplayGifs && !appearance.reducedMotion))fetch()}
    DisposableEffect(file) {onDispose {url?.let(::browserRevokeFileUrl)}}
    Column(modifier,verticalArrangement=Arrangement.spacedBy(8.dp)) {
        val current=url
        if(current!=null && media) {
            if(expanded && file.type.startsWith("image/"))Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                Symbol("remove","Zoom out",{zoom=(zoom-1).coerceAtLeast(1)})
                Text("${zoom*100}%",style=MaterialTheme.typography.labelMedium)
                Symbol("add","Zoom in",{zoom=(zoom+1).coerceAtMost(8)})
                SigilTextButton({zoom=1}){Text("Fit")}
            }
            WebMedia(current,file.type,file.name,Modifier.fillMaxWidth().height(if(file.type.startsWith("audio/"))64.dp else if(expanded)480.dp else 200.dp),zoom)
        }
        else Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
            Glyph(if(media)"play_circle" else "description",32)
            Column(Modifier.weight(1f)) {Text(file.name,style=MaterialTheme.typography.titleSmall);Text("${file.bytes/1024} KiB",style=MaterialTheme.typography.labelSmall)}
        }
        if(loading)LinearProgressIndicator(Modifier.fillMaxWidth())
        issue?.let {Text(it,style=MaterialTheme.typography.bodySmall)}
        Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            if(current==null)SigilTextButton({scope.launch {fetch()}},enabled=!loading && file.bytes<=128*1024*1024){Text(if(issue!=null)"Retry" else "Load attachment",color=ink)}
            if(!expanded)SigilTextButton({open(file)}){Text("Expand",color=ink)}
            if(browserFileStreamSupported())SigilTextButton({
                val destination=browserFileDestination(file.name)
                scope.launch {saving=true;try{destination.await<JsAny?>()?.let {save(file,it)}}catch(cancelled:CancellationException){throw cancelled}catch(_:Exception){issue="Could not save this attachment. Check your connection and available space."}finally{saving=false}}
            },enabled=!saving){Text(if(saving)"Saving…" else "Save file",color=ink)}
            else if(current!=null)SigilTextButton({browserSaveFileUrl(current,file.name)}){Text("Save file",color=ink)}
        }
        if(file.bytes>128*1024*1024)Text(if(browserFileStreamSupported())"Save this file to open it. Inline viewing supports up to 128 MiB." else "This browser supports saving up to 128 MiB. Use a browser with file-system access or the Android app for larger files.",style=MaterialTheme.typography.bodySmall)
        if(expanded && file.caption.isNotEmpty())Text(file.caption)
    }
}

@Composable private fun WebMedia(url:String,type:String,name:String,modifier:Modifier,zoom:Int) {
    val element=remember(url) {
        val tag=when {type.startsWith("image/")->"img";type.startsWith("audio/")->"audio";else->"video"}
        (document.createElement(tag) as HTMLElement).apply {
            setAttribute("src",url);setAttribute("aria-label",name)
            setAttribute("style","display:block;width:100%;height:100%;object-fit:contain;border-radius:16px")
            if(tag=="img")setAttribute("alt",name)
            else {setAttribute("controls","");setAttribute("preload","metadata");setAttribute("playsinline","")}
        }
    }
    val viewport=remember(element){(document.createElement("div") as HTMLElement).apply {
        setAttribute("style","width:100%;height:100%;overflow:auto;overscroll-behavior:contain;border-radius:16px")
        setAttribute("tabindex","0");setAttribute("aria-label","Attachment viewer");appendChild(element)
    }}
    WebElementView(factory={viewport},modifier=modifier,update={
        val size=if(type.startsWith("image/"))zoom*100 else 100
        element.style.width="$size%";element.style.height="$size%"
    },onRelease={
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
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).verticalScroll(rememberScrollState()).padding(24.dp),verticalArrangement=Arrangement.spacedBy(16.dp)) {
        Row(Modifier.fillMaxWidth(),verticalAlignment=Alignment.CenterVertically) {
            Symbol("chevron_left","Back",close)
            Text(file.name,Modifier.weight(1f),style=MaterialTheme.typography.titleLarge)
        }
        WebAttachment(file,load,{},Modifier.widthIn(max=1000.dp).fillMaxWidth().align(Alignment.CenterHorizontally),expanded=true)
    }
}
