@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class,androidx.compose.ui.ExperimentalComposeUiApi::class)
package org.sigil

import androidx.compose.runtime.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.WebElementView
import kotlinx.browser.document
import kotlinx.coroutines.*
import org.w3c.dom.HTMLVideoElement
import kotlin.js.*

@Composable internal fun WebQrScanner(found:(String)->Unit) {
    val callback by rememberUpdatedState(found)
    var error by remember {mutableStateOf<String?>(null)}
    var attached by remember {mutableStateOf(false)}
    var attempt by remember {mutableStateOf(0)}
    val video=remember {(document.createElement("video") as HTMLVideoElement).apply {id="sigil-link-camera";setAttribute("style","display:block;width:100%;height:100%;object-fit:cover;border-radius:20px;background:#111");setAttribute("aria-label","Device linking camera preview")}}
    WebElementView(factory={video},modifier=Modifier.fillMaxWidth().aspectRatio(1f),update={attached=true},onRelease={browserCameraStop()})
    LaunchedEffect(attached,attempt) {
        if(attached)try {
            browserCameraStart(video).await<JsAny?>()
            while(isActive) {delay(350);browserCameraScan()?.let {browserCameraStop();callback(it);return@LaunchedEffect}}
        }catch(cancelled:CancellationException){throw cancelled}
        catch(_:Exception){error="Could not open the camera. Allow camera access, then retry."}
    }
    DisposableEffect(Unit){onDispose{browserCameraStop()}}
    error?.let {Text(it);SigilTextButton({error=null;attempt++}){Text("Retry camera")}}
}
