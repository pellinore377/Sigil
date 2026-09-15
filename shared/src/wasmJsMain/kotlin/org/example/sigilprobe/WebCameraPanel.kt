@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class,androidx.compose.ui.ExperimentalComposeUiApi::class)
package org.sigil

import androidx.compose.runtime.*
import androidx.compose.foundation.layout.*
import androidx.compose.ui.Modifier
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
    var front by remember {mutableStateOf(false)}
    var attempt by remember {mutableStateOf(0)}
    val scope=rememberCoroutineScope()
    val video=remember {(document.createElement("video") as HTMLVideoElement).apply {setAttribute("style","display:block;width:100%;height:100%;object-fit:cover;background:#111");setAttribute("aria-label","Camera preview")}}
    LaunchedEffect(attempt,front) {
        ready=false;issue=null
        try {browserCameraStartPhoto(video,front).awaitBrowser<JsAny?>();ready=true}
        catch(cancelled:CancellationException){throw cancelled}
        catch(_:Exception){issue="Allow camera access, then retry."}
    }
    DisposableEffect(Unit){onDispose {browserCameraStop();url?.let(::browserRevokeFileUrl)}}
    Column(Modifier.fillMaxSize()) {
        WebCameraView(video,url,ready,busy,issue,Modifier.weight(1f).fillMaxWidth(),close=back,capture={
            scope.launch {busy=true;try {val captured=browserCameraPhoto().awaitBrowser<JsAny>();file=captured;url=browserCameraPhotoUrl(captured);browserCameraStop()}
                catch(cancelled:CancellationException){throw cancelled}catch(_:Exception){issue="Could not capture this photo. Retry."}finally{busy=false}}
        },retake={url?.let(::browserRevokeFileUrl);url=null;file=null;attempt++},flip={front=!front},retry={attempt++})
        if(file!=null)BuilderConfirm("Attach photo",!busy) {scope.launch {busy=true;try {use(file!!)}catch(cancelled:CancellationException){throw cancelled}catch(_:Exception){issue="Could not attach this photo. Retry."}finally{busy=false}}}
    }
}
