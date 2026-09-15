@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class,androidx.compose.ui.ExperimentalComposeUiApi::class)
package org.sigil

import androidx.compose.runtime.*
import androidx.compose.foundation.layout.Box
import androidx.compose.material3.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.viewinterop.WebElementView
import kotlinx.browser.document
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.platform.LocalDensity
import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import org.w3c.dom.HTMLCanvasElement

internal object WebMaterials:MaterialPlatform {
    override val available=true
    @Composable override fun Object(kind:Int,sides:Int,face:Int,rotation:FloatArray?,label:String?,modifier:Modifier,progress:Float) {
        val timeline = LocalMaterialTimeline.current.takeUnless { LocalObjectMenu.current }
        val occlusion = LocalMaterialOcclusion.current
        val launchWindow by rememberUpdatedState(LocalMaterialLaunchWindow.current)
        val density = LocalDensity.current.density
        val appearance=LocalAppearance.current
        val accent=(if(appearance.objectMode=="Global")LocalGlobalAccent.current else MaterialTheme.colorScheme.primary).toArgb()
        val style=remember(appearance,kind){appearance.objectStyle(kind).parameters(appearance.objectMode)}
        val visible=LocalMotionVisible.current
        val frame=buildJsonObject {
            put("kind",kind);put("sides",sides);put("face",face);put("font",if(appearance.font=="Google Sans Flex")1 else 0)
            put("accent",accent.toUInt().toLong());put("label",label.orEmpty());put("progress",progress)
            put("rotation",rotation?.let {JsonArray(it.map(::JsonPrimitive))} ?: JsonNull)
            put("style",JsonArray(style.map(::JsonPrimitive)))
        }.toString()
        var failed by remember {mutableStateOf(false)}
        val canvas=remember {(document.createElement("canvas") as HTMLCanvasElement).apply {width=if(kind==2)130 else 192;height=192;setAttribute("style","display:block;width:100%;height:100%;pointer-events:none");setAttribute("data-sigil-material","canvas");if(timeline!=null)this.style.setProperty("clip-path","inset(100%)")}}
        LaunchedEffect(canvas, timeline, occlusion, density, visible) {
            if (timeline == null) { canvas.style.removeProperty("clip-path"); return@LaunchedEffect }
            var previous = ""
            while (isActive && visible) {
                withFrameNanos { }
                webInteropPointerPassThrough(canvas)
                val bounds = occlusion?.visible(timeline.viewport) ?: timeline.viewport
                val clip = Rect(bounds.left / density, bounds.top / density, bounds.right / density, bounds.bottom / density)
                val element = canvas.getBoundingClientRect()
                val insets = materialClipInsets(clip, element.left.toFloat(), element.top.toFloat(), element.width.toFloat(), element.height.toFloat())
                val launch = occlusion?.launch(timeline.viewport, launchWindow)?.let { Rect(it.left / density, it.top / density, it.right / density, it.bottom / density) }
                val notice=occlusion?.notice?.takeIf {it.width>0 && it.height>0}?.let {Rect(it.left/density,it.top/density,it.right/density,it.bottom/density)}
                val value = if (launch == null && notice == null) "inset(" + insets.joinToString(" ") { "${it}%" } + ")"
                    else materialClipPath(clip, launch, element.left.toFloat(), element.top.toFloat(), notice)
                if (value != previous) { canvas.style.setProperty("clip-path", value); previous = value }
            }
        }
        val opacity=LocalMaterialOpacity.current
        SideEffect {canvas.style.opacity=opacity.coerceIn(0f,1f).toString()}
        val holder=remember {arrayOf<BrowserMaterialView?>(null)}
        DisposableEffect(Unit){onDispose{holder[0]?.free();holder[0]=null}}
        LaunchedEffect(frame,visible) {
            if(visible && !failed)try {
                val view=holder[0] ?: BrowserMaterialView(canvas).also{holder[0]=it}
                withTimeout(5000){while(!view.draw(frame)){delay(16)}}
            }catch(cancelled:CancellationException){if(cancelled !is TimeoutCancellationException)throw cancelled;failed=true}
            catch(_:Exception){failed=true}
            if(failed){holder[0]?.free();holder[0]=null}
        }
        if(failed)Box(modifier,contentAlignment=Alignment.Center){Text(label?.takeIf {it.isNotEmpty()} ?: face.toString())}
        else WebElementView(factory={canvas},modifier=modifier)
    }

    override suspend fun record(data:FloatArray):FloatArray? {
        yield()
        return try {Json.parseToJsonElement(browserMaterialRecord(JsonArray(data.map(::JsonPrimitive)).toString()).awaitBrowser<kotlin.js.JsString>().toString()).jsonArray.map{it.jsonPrimitive.float}.toFloatArray()}catch(cancelled:CancellationException){throw cancelled}catch(_:Exception){null}
    }
    override fun horizontalExtent(sides:Int,face:Int,rotation:FloatArray?,outgoing:Boolean)=runCatching {browserMaterialExtent(sides,face,rotation?.let {JsonArray(it.map(::JsonPrimitive)).toString()}.orEmpty(),outgoing)}.getOrDefault(1.14f)
}
