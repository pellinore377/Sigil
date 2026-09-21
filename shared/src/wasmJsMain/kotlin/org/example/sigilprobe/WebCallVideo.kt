@file:OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
package org.sigil

import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.WebElementView
import kotlinx.browser.document
import org.w3c.dom.HTMLCanvasElement
import kotlinx.coroutines.delay

/** Self view shows the live camera element; peers draw decoded frames onto a canvas. */
@Composable internal fun WebCallVideo(calls:WebCalls,member:String,screen:Boolean,modifier:Modifier) {
    var aspect by remember(member,screen) { mutableStateOf(16f/9f) }
    if(member=="self"){
        LaunchedEffect(calls){while(true){val video=calls.selfVideo;if(video.videoHeight>0)aspect=video.videoWidth.toFloat()/video.videoHeight;delay(250)}}
        CallVideoFrame(aspect,true,modifier){WebElementView(factory={calls.selfVideo},modifier=it)}
        return
    }
    val canvas=remember(member,screen){(document.createElement("canvas") as HTMLCanvasElement).apply {setAttribute("style","display:block;width:100%;height:100%;object-fit:contain;background:#000");setAttribute("aria-label","Call video")}}
    DisposableEffect(member,screen){runCatching{browserVideoAttach(member,canvas)};onDispose{browserVideoDetach(member)}}
    LaunchedEffect(canvas){while(true){if(canvas.height>0)aspect=canvas.width.toFloat()/canvas.height;delay(250)}}
    CallVideoFrame(aspect,false,modifier){WebElementView(factory={canvas},modifier=it)}
}
