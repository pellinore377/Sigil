@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class,androidx.compose.ui.ExperimentalComposeUiApi::class)
package org.sigil

import androidx.compose.runtime.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.WebElementView
import kotlinx.browser.document
import kotlinx.coroutines.*
import org.w3c.dom.*
import kotlin.js.*

@Composable internal fun WebCameraPanel(back:()->Unit,use:suspend(JsAny)->Unit) {
    var file by remember {mutableStateOf<JsAny?>(null)}
    var url by remember {mutableStateOf<String?>(null)}
    var issue by remember {mutableStateOf<String?>(null)}
    var busy by remember {mutableStateOf(false)}
    var ready by remember {mutableStateOf(false)}
    var attempt by remember {mutableStateOf(0)}
    val scope=rememberCoroutineScope()
    val video=remember {(document.createElement("video") as HTMLVideoElement).apply {setAttribute("style","display:block;width:100%;height:100%;object-fit:contain;border-radius:16px;background:#111");setAttribute("aria-label","Camera preview")}}
    LaunchedEffect(attempt) {
        ready=false;issue=null
        try {browserCameraStartPhoto(video).awaitBrowser<JsAny?>();ready=true}
        catch(cancelled:CancellationException){throw cancelled}
        catch(_:Exception){issue="Allow camera access, then retry."}
    }
    DisposableEffect(Unit){onDispose {browserCameraStop();url?.let(::browserRevokeFileUrl)}}
    Column(Modifier.fillMaxSize().padding(12.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(Modifier.fillMaxWidth(),verticalAlignment=Alignment.CenterVertically) {Symbol("chevron_left","Back",back);Text("Camera",style=MaterialTheme.typography.titleMedium)}
        val snapshot=url
        if(snapshot==null)WebElementView(factory={video},modifier=Modifier.weight(1f).fillMaxWidth())
        else key(snapshot) {WebElementView(factory={(document.createElement("img") as HTMLElement).apply {setAttribute("src",snapshot);setAttribute("alt","Photo preview");setAttribute("style","display:block;width:100%;height:100%;object-fit:contain;border-radius:16px")}},modifier=Modifier.weight(1f).fillMaxWidth())}
        issue?.let {Text(it,style=MaterialTheme.typography.bodySmall)}
        Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.Center) {
            if(file!=null) {
                SigilTextButton({url?.let(::browserRevokeFileUrl);url=null;file=null;attempt++},enabled=!busy){Text("Retake")}
                SigilButton({scope.launch {busy=true;try {use(file!!)}catch(cancelled:CancellationException){throw cancelled}catch(_:Exception){issue="Could not attach this photo. Retry."}finally{busy=false}}},enabled=!busy){Text("Use photo")}
            }else if(issue!=null)SigilTextButton({attempt++}){Text("Retry camera")}
            else SigilButton({scope.launch {busy=true;try {val captured=browserCameraPhoto().awaitBrowser<JsAny>();file=captured;url=browserCameraPhotoUrl(captured);browserCameraStop()}catch(_:Exception){issue="Could not capture this photo. Retry."}finally{busy=false}}},enabled=ready&&!busy){Text("Take photo")}
        }
    }
}
