@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class,androidx.compose.ui.ExperimentalComposeUiApi::class)
package org.sigil

import androidx.compose.runtime.*
import androidx.compose.material3.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.viewinterop.WebElementView
import kotlinx.browser.document
import org.w3c.dom.HTMLElement

@Composable internal fun WebMath(mathml:String,expression:String,modifier:Modifier) {
    val color=LocalContentColor.current.toArgb().and(0xffffff).toString(16).padStart(6,'0')
    var failed by remember(mathml){mutableStateOf(false)}
    val node=remember(mathml) {(document.createElement("div") as HTMLElement).also {
        it.setAttribute("aria-label",expression)
        if(runCatching {browserRenderMath(it,mathml)}.isFailure)failed=true
    }}
    if(failed)Text(expression,modifier)
    else WebElementView(factory={node},modifier=modifier,update={it.setAttribute("style","display:flex;align-items:center;justify-content:center;width:100%;height:100%;overflow:auto;color:#$color;font-size:24px")})
}
