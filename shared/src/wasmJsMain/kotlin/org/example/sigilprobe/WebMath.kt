@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class,androidx.compose.ui.ExperimentalComposeUiApi::class)
package org.sigil

import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.*
import androidx.compose.material3.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.WebElementView
import kotlinx.browser.document
import org.w3c.dom.HTMLElement

// Sized from the laid-out formula at the caller's text size; a wide one shrinks to fit rather than scroll.
@Composable internal fun WebMath(mathml:String,expression:String,modifier:Modifier) {
    val color=LocalContentColor.current.toArgb().and(0xffffff).toString(16).padStart(6,'0')
    val density=LocalDensity.current
    val font=with(density) {LocalTextStyle.current.fontSize.takeIf {it.isSp}?.toPx()?.div(density.density) ?: 24f}
    var failed by remember(mathml){mutableStateOf(false)}
    var node by remember(mathml){mutableStateOf<HTMLElement?>(null)}
    LaunchedEffect(mathml) {
        val target=document.createElement("div") as HTMLElement
        target.setAttribute("aria-label",expression)
        fun draw()=runCatching {browserRenderMath(target,mathml)}.isSuccess
        // Surfaces without the messenger (the workbench) load the browser module on first use.
        if(draw() || (runCatching {initializeBrowser().awaitBrowser<JsAny?>()}.isSuccess && draw())) {
            // The UA sheet sets the face on <math> itself, so it cannot be inherited.
            target.firstElementChild?.setAttribute("style","font-family:$MathFaces")
            node=target
        } else failed=true
    }
    val natural=remember(node,font) {node?.let {measureMath(it,font)}}
    if(failed || (node!=null && natural==null)) {Text(expression,modifier,style=MaterialTheme.typography.bodyMedium.copy(fontFamily=LocalCodeFont.current));return}
    val shown=node ?: return
    BoxWithConstraints(modifier) {
        val room=if(constraints.hasBoundedWidth)maxWidth.value else natural!!.first
        val tall=if(constraints.hasBoundedHeight)maxHeight.value/natural!!.second else 1f
        val scale=minOf(room/natural!!.first,tall).coerceIn(.5f,1f)
        WebElementView(factory={shown},modifier=Modifier.size((natural.first*scale).dp,(natural.second*scale).dp),
            update={it.setAttribute("style","display:flex;align-items:center;justify-content:center;width:100%;height:100%;overflow:hidden;pointer-events:none;color:#$color;font-size:${font*scale}px")})
    }
}

// Serif math faces sit beside Newsreader; the platform's own math font is the floor.
private const val MathFaces="'STIX Two Math','Latin Modern Math','Cambria Math','Noto Sans Math',math"

private fun measureMath(node:HTMLElement,font:Float):Pair<Float,Float>? {
    val probe=node.cloneNode(true) as HTMLElement
    probe.setAttribute("style","position:absolute;left:-10000px;top:0;visibility:hidden;display:inline-block;white-space:nowrap;font-size:${font}px")
    // Not body: Compose gives it a shadow root, so light children there never lay out.
    document.documentElement?.appendChild(probe) ?: return null
    val box=probe.getBoundingClientRect()
    probe.remove()
    val width=kotlin.math.ceil(box.width).toFloat()+2f
    val height=kotlin.math.ceil(box.height).toFloat()+2f
    return if(width<=2f || height<=2f)null else width to height
}
