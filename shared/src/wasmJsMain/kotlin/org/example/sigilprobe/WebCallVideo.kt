@file:OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
package org.sigil

import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.WebElementView
import kotlinx.browser.document
import org.w3c.dom.HTMLCanvasElement

/** Self view shows the live camera element; peers draw decoded frames onto a canvas. */
@Composable internal fun WebCallVideo(calls:WebCalls,member:String,screen:Boolean,modifier:Modifier) {
    if(member=="self"){WebElementView(factory={calls.selfVideo},modifier=modifier);return}
    val canvas=remember(member,screen){(document.createElement("canvas") as HTMLCanvasElement).apply {setAttribute("style","display:block;width:100%;height:100%;object-fit:cover;background:#000");setAttribute("aria-label","Call video")}}
    DisposableEffect(member,screen){runCatching{browserVideoAttach(member,canvas)};onDispose{browserVideoDetach(member)}}
    WebElementView(factory={canvas},modifier=modifier)
}
