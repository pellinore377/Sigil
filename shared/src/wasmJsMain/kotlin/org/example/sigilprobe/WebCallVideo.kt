@file:OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
package org.sigil

import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.WebElementView
import kotlinx.browser.document
import org.w3c.dom.HTMLCanvasElement
import org.w3c.dom.HTMLVideoElement
import org.w3c.dom.HTMLElement
import kotlinx.coroutines.delay

/** Self view shows the live camera element; peers draw decoded frames onto a canvas. */
@Composable internal fun WebCallVideo(calls:WebCalls,member:String,screen:Boolean,modifier:Modifier) {
    var aspect by remember(member,screen) { mutableStateOf(16f/9f) }
    if(member=="self"){
        LaunchedEffect(calls){while(true){val video=calls.selfVideo;if(video.videoHeight>0)aspect=video.videoWidth.toFloat()/video.videoHeight;delay(250)}}
        CallVideoFrame(aspect,true,modifier){WebElementView(factory={calls.selfVideo},modifier=it)}
        return
    }
    if(!screen){
        val video=remember(member){document.createElement("video") as HTMLVideoElement}
        val holder=remember(video){(document.createElement("div") as HTMLElement).apply{setAttribute("style","width:100%;height:100%;position:relative;overflow:hidden;border-radius:20px");setAttribute("aria-label","Call video");appendChild(video)}}
        DisposableEffect(member,video){browserVideoNativeAttach(member,video);onDispose{browserVideoNativeDetach(member)}}
        LaunchedEffect(video){
            var previous=""
            while(true){
                val rotation=video.getAttribute("data-rotation")?.toIntOrNull()?:0
                val width=video.getAttribute("data-width")?.toIntOrNull()?:video.videoWidth
                val height=video.getAttribute("data-height")?.toIntOrNull()?:video.videoHeight
                val shape="$rotation/$width/$height"
                if(width>0&&height>0&&shape!=previous){
                    val sideways=rotation%180!=0
                    aspect=if(sideways)height.toFloat()/width else width.toFloat()/height
                    video.setAttribute("style","position:absolute;left:50%;top:50%;width:${if(sideways)100f/aspect else 100f}%;height:${if(sideways)100f*aspect else 100f}%;object-fit:contain;transform:translate(-50%,-50%) rotate(${rotation}deg)")
                    previous=shape
                }
                delay(250)
            }
        }
        CallVideoFrame(aspect,false,modifier){WebElementView(factory={holder},modifier=it)}
        return
    }
    val canvas=remember(member,screen){(document.createElement("canvas") as HTMLCanvasElement).apply {setAttribute("style","display:block;width:100%;height:100%;object-fit:contain;background:#000");setAttribute("aria-label","Call video")}}
    DisposableEffect(member,screen){runCatching{browserVideoAttach(member,canvas)};onDispose{browserVideoDetach(member)}}
    LaunchedEffect(canvas){while(true){if(canvas.height>0)aspect=canvas.width.toFloat()/canvas.height;delay(250)}}
    CallVideoFrame(aspect,false,modifier){WebElementView(factory={canvas},modifier=it)}
}
