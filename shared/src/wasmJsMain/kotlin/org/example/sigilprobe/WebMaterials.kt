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
        val active=LocalMotionVisible.current
        var inViewport by remember {mutableStateOf(timeline==null)}
        val visible=active && inViewport
        val frame=buildJsonObject {
            put("kind",kind);put("sides",sides);put("face",face);put("font",if(appearance.font=="Google Sans Flex")1 else 0)
            put("accent",accent.toUInt().toLong());put("label",label.orEmpty());put("progress",progress)
            put("rotation",rotation?.let {JsonArray(it.map(::JsonPrimitive))} ?: JsonNull)
            put("style",JsonArray(style.map(::JsonPrimitive)))
        }.toString()
        var failed by remember {mutableStateOf(false)}
        val canvas=remember {(document.createElement("canvas") as HTMLCanvasElement).apply {width=if(kind==2)130 else 192;height=192;setAttribute("style","display:block;width:100%;height:100%;pointer-events:none");setAttribute("data-sigil-material","canvas");if(timeline!=null)this.style.setProperty("clip-path","inset(100%)")}}
        LaunchedEffect(canvas, timeline, occlusion, density, active) {
            if (timeline == null) { canvas.style.removeProperty("clip-path"); return@LaunchedEffect }
            var previous = ""
            while (isActive && active) {
                withFrameNanos { }
                webInteropPointerPassThrough(canvas)
                val bounds = occlusion?.visible(timeline.viewport) ?: timeline.viewport
                val clip = Rect(bounds.left / density, bounds.top / density, bounds.right / density, bounds.bottom / density)
                val element = canvas.getBoundingClientRect()
                val insets = materialClipInsets(clip, element.left.toFloat(), element.top.toFloat(), element.width.toFloat(), element.height.toFloat())
                val launch = occlusion?.launch(timeline.viewport, launchWindow)?.let { Rect(it.left / density, it.top / density, it.right / density, it.bottom / density) }
                val covered=occlusion?.covered(timeline.viewport).orEmpty().filter {it.width>0 && it.height>0}
                    .map {Rect(it.left/density,it.top/density,it.right/density,it.bottom/density)}
                val elementBounds=Rect(element.left.toFloat(),element.top.toFloat(),element.right.toFloat(),element.bottom.toFloat())
                // Retain the surface anywhere near the timeline; only drawing is clipped.
                val margin=clip.height.coerceAtLeast(1f)
                inViewport=Rect(clip.left,clip.top-margin,clip.right,clip.bottom+margin).overlaps(elementBounds) ||
                    launch?.overlaps(elementBounds)==true
                val value = if (launch == null && covered.isEmpty()) "inset(" + insets.joinToString(" ") { "${it}%" } + ")"
                    else materialClipPath(clip, launch, element.left.toFloat(), element.top.toFloat(), covered)
                if (value != previous) { canvas.style.setProperty("clip-path", value); previous = value }
            }
        }
        val opacity=LocalMaterialOpacity.current
        SideEffect {canvas.style.opacity=opacity.coerceIn(0f,1f).toString()}
        val holder=remember {arrayOf<BrowserMaterialView?>(null)}
        val rendered=remember {arrayOf<String?>(null)}
        DisposableEffect(Unit){onDispose{holder[0]?.free();holder[0]=null}}
        LaunchedEffect(frame,visible) {
            if(!visible) {holder[0]?.free();holder[0]=null}
            if(visible && !failed && rendered[0]!=frame)try {
                // Handoff and departing LazyColumn rows can temporarily occupy the
                // bounded GPU pool. Cancellation stops waiting when this row leaves.
                while(holder[0]==null && !browserMaterialViewAvailable())delay(100)
                val view=holder[0] ?: BrowserMaterialView(canvas).also{holder[0]=it}
                withTimeout(5000){while(!view.draw(frame)){delay(16)}}
                rendered[0]=frame
                // The display canvas keeps exact final pixels; the bounded GPU
                // pool belongs to moving/new objects, not settled history rows.
                if(progress>=1f){view.free();holder[0]=null}
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
