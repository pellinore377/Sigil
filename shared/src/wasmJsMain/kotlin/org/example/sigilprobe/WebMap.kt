@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class,androidx.compose.ui.ExperimentalComposeUiApi::class)
package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.viewinterop.WebElementView
import androidx.compose.ui.unit.dp
import kotlinx.browser.document
import kotlinx.browser.window
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*
import org.w3c.dom.*
import org.w3c.dom.events.*
import kotlin.math.*
import kotlin.js.*

@JsName("Date") private external class MapDate : JsAny { companion object { fun now():Double } }

private val mapOwner = Mutex()
private val mapOwners = WebMapOwners()

@Composable internal fun WebMap(latitude:Double,longitude:Double,modifier:Modifier=Modifier,showMarker:Boolean=true,pin:((Double,Double)->Unit)?=null,avatar:(@Composable ()->Unit)?=null,controls:Boolean=true,snapshot:Boolean=false,recenter:Int=0,placeMarker:Boolean=false,center:Pair<Double,Double>?=null,focused:Boolean=showMarker) {
    val owner=remember {Any()}
    DisposableEffect(owner,snapshot){if(!snapshot)mapOwners.attach(owner);onDispose{if(!snapshot)mapOwners.detach(owner)}}
    val isOwner=if(snapshot)mapOwners.current==null else mapOwners.current === owner
    val dark=MaterialTheme.colorScheme.surface.luminance()<.5f
    val sans=LocalAppearance.current.font!="Newsreader"
    val currentPin by rememberUpdatedState(pin)
    var lat by remember {mutableDoubleStateOf(latitude)}
    var lon by remember {mutableDoubleStateOf(longitude)}
    var zoom by remember {mutableIntStateOf(13)}
    var minimum by remember {mutableIntStateOf(0)}
    var maximum by remember {mutableIntStateOf(18)}
    var revision by remember {mutableIntStateOf(0)}
    var viewport by remember {mutableStateOf(280 to 160)}
    var renderer by remember {mutableStateOf<WebMapRenderer?>(null)}
    var issue by remember {mutableStateOf<String?>(null)}
    var loading by remember {mutableStateOf(true)}
    var retry by remember {mutableIntStateOf(0)}
    var marker by remember {mutableStateOf<Pair<Double,Double>?>(null)}
    var renderedSnapshot by remember {mutableStateOf<List<Any>?>(null)}
    val canvas=remember {(document.createElement("canvas") as HTMLCanvasElement).apply {setAttribute("style","display:block;width:100%;height:100%;touch-action:none;border-radius:20px");setAttribute("aria-label",if(snapshot)"Shared location map" else "Map. Drag to pan; use zoom controls.");if(snapshot)style.setProperty("pointer-events","none")}}
    val preview=remember {(document.createElement("img") as HTMLImageElement).apply {
        setAttribute("style","display:block;width:100%;height:100%;pointer-events:none;border-radius:20px")
        alt="Shared location map"
    }}
    LaunchedEffect(latitude,longitude,recenter,center) {lat=center?.first ?: latitude;lon=center?.second ?: longitude}
    LaunchedEffect(focused,recenter,center) {if(focused)zoom=13.coerceIn(minimum,maximum)}
    DisposableEffect(canvas) {
        var start:Pair<Double,Double>?=null
        val down:(Event)->Unit={e->(e as? MouseEvent)?.let {start=it.clientX.toDouble() to it.clientY.toDouble();e.preventDefault()}}
        val up:(Event)->Unit={e->val mouse=e as? MouseEvent;val begin=start;start=null;val map=renderer
            if(!snapshot && mouse!=null && begin!=null && map!=null){val dx=mouse.clientX-begin.first;val dy=mouse.clientY-begin.second
                val bounds=canvas.getBoundingClientRect()
                if(abs(dx)+abs(dy)>5){val p=Json.parseToJsonElement(map.point((bounds.width/2-dx)*viewport.first/bounds.width,(bounds.height/2-dy)*viewport.second/bounds.height)).jsonObject;lat=p["lat"]!!.jsonPrimitive.double;lon=p["lon"]!!.jsonPrimitive.double}
                else if(currentPin!=null){val p=Json.parseToJsonElement(map.point((mouse.clientX-bounds.left)*viewport.first/bounds.width,(mouse.clientY-bounds.top)*viewport.second/bounds.height)).jsonObject;currentPin?.invoke(p["lat"]!!.jsonPrimitive.double,p["lon"]!!.jsonPrimitive.double)}
            }}
        val resize:(Event)->Unit={revision++}
        canvas.addEventListener("pointerdown",down);window.addEventListener("pointerup",up);window.addEventListener("resize",resize)
        onDispose {canvas.removeEventListener("pointerdown",down);window.removeEventListener("pointerup",up);window.removeEventListener("resize",resize)}
    }
    LaunchedEffect(dark,sans,isOwner,retry,if(snapshot)latitude else null,if(snapshot)longitude else null,if(snapshot)viewport else null) {
        if(!isOwner){loading=false;return@LaunchedEffect}
        val snapshotKey:List<Any> = listOf(dark,sans,latitude,longitude,retry,viewport)
        if(snapshot && renderedSnapshot==snapshotKey)return@LaunchedEffect
        loading=true;issue=null
        if(snapshot){preview.removeAttribute("src");marker=null}
        mapOwner.withLock {
        var owned:WebMapRenderer?=null
        try {
            initializeMapsWasm().awaitBrowser<JsAny>();ensureActive()
            owned=withContext(NonCancellable){createWebMap(canvas,dark,sans).awaitBrowser<WebMapRenderer>()}
            ensureActive()
            owned.configure(browserMapResource("/client/v0/maps/style.json").awaitBrowser<JsAny>())
            val metadata=Json.parseToJsonElement(owned.metadata(browserMapResource("/client/v0/maps/tiles.json").awaitBrowser<JsAny>())).jsonObject
            minimum=metadata["min"]!!.jsonPrimitive.int;maximum=metadata["max"]!!.jsonPrimitive.int
            zoom=zoom.coerceIn(minimum,maximum)
            if(!focused){lat=metadata["lat"]!!.jsonPrimitive.double;lon=metadata["lon"]!!.jsonPrimitive.double;zoom=5.coerceIn(minimum,maximum)}
            if(snapshot) {
                val tiles=Json.parseToJsonElement(owned.view(latitude,longitude,13.coerceIn(minimum,maximum),viewport.first,viewport.second,window.devicePixelRatio.coerceIn(1.0,2.0))).jsonArray
                for(value in tiles) {
                    ensureActive()
                    val tile=value.jsonObject;val z=tile["z"]!!.jsonPrimitive.int;val x=tile["x"]!!.jsonPrimitive.int;val y=tile["y"]!!.jsonPrimitive.int
                    owned.tile(z,x,y,browserMapResource("/client/v0/maps/tiles/$z/$x/$y").awaitBrowser<JsAny>())
                }
                preview.src=owned.snapshot()
                marker=viewport.first/2.0 to viewport.second/2.0
                renderedSnapshot=snapshotKey
                loading=false
            } else {renderer=owned;awaitCancellation()}
        }catch(cancelled:CancellationException){throw cancelled}
        catch(error:Exception){issue="Map unavailable.";loading=false}
        finally {renderer=null;owned?.free()}
        }
    }
    BoxWithConstraints(modifier.clip(RoundedCornerShape(20.dp)).background(MaterialTheme.colorScheme.surfaceContainer)) {
        val scale=maxOf(1f,maxWidth.value/1024f,maxHeight.value/768f)
        val width=(maxWidth.value/scale).roundToInt().coerceIn(1,1024)
        val height=(maxHeight.value/scale).roundToInt().coerceIn(1,768)
        SideEffect {viewport=width to height}
        LaunchedEffect(renderer,lat,lon,zoom,width,height,revision) {
            val map=renderer ?: return@LaunchedEffect
            loading=true;issue=null
            try {
                val tiles=Json.parseToJsonElement(map.view(lat,lon,zoom,width,height,window.devicePixelRatio.coerceIn(1.0,2.0))).jsonArray
                val p=Json.parseToJsonElement(map.project(latitude,longitude)).jsonObject;marker=p["x"]!!.jsonPrimitive.double to p["y"]!!.jsonPrimitive.double
                delay(100)
                for(value in tiles){ensureActive();val tile=value.jsonObject;val z=tile["z"]!!.jsonPrimitive.int;val x=tile["x"]!!.jsonPrimitive.int;val y=tile["y"]!!.jsonPrimitive.int
                    val data=browserMapResource("/client/v0/maps/tiles/$z/$x/$y").awaitBrowser<JsAny>();ensureActive();if(renderer !== map)return@LaunchedEffect;map.tile(z,x,y,data)
                }
            }catch(cancelled:CancellationException){throw cancelled}
            catch(_:Exception){issue="This map area could not be loaded."}
            finally {loading=false}
        }
        WebElementView(factory={if(snapshot)preview else canvas},modifier=Modifier.fillMaxSize())
        val anchor=if(snapshot && marker!=null)width/2.0 to height/2.0 else marker
        if(showMarker)anchor?.let {(x,y)->if(x in 0.0..width.toDouble() && y in 0.0..height.toDouble()) {
            val screenX=x/width*maxWidth.value;val screenY=y/height*maxHeight.value
            if(avatar!=null)Box(Modifier.offset((screenX-32).dp,(screenY-32).dp).size(64.dp)){avatar()}
            else Surface(Modifier.offset((screenX-16).dp,(screenY-16).dp).size(32.dp),shape=RoundedCornerShape(16.dp),color=MaterialTheme.colorScheme.primary){Box(contentAlignment=Alignment.Center){Glyph(if(pin!=null || placeMarker)"place" else "my_location",22)}}
        }}
        if(controls && !snapshot) Column(Modifier.align(Alignment.BottomEnd).padding(8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
            Surface(shape=RoundedCornerShape(16.dp),color=MaterialTheme.colorScheme.surface){Column {Symbol("add","Zoom in",{zoom=(zoom+1).coerceAtMost(maximum)});Symbol("remove","Zoom out",{zoom=(zoom-1).coerceAtLeast(minimum)})}}
        }
        if(loading)CircularProgressIndicator(Modifier.align(Alignment.TopStart).padding(12.dp).size(20.dp),strokeWidth=2.dp)
        issue?.let {Surface(Modifier.align(Alignment.Center).padding(16.dp),shape=RoundedCornerShape(16.dp)){Column(Modifier.padding(12.dp),horizontalAlignment=Alignment.CenterHorizontally){Text(it,style=MaterialTheme.typography.bodySmall);if(!snapshot)SigilTextButton({retry++}){Text("Retry")}}}}
        Surface(Modifier.align(Alignment.BottomStart),color=MaterialTheme.colorScheme.surface.copy(alpha=.9f)){Text("© OpenStreetMap · Protomaps",Modifier.padding(6.dp),style=MaterialTheme.typography.labelSmall)}
    }
}

@Composable internal fun WebLocationCard(message:ChatMessage,part:MessagePart,name:String,command:Command?) {
    var expanded by remember(message.id,part.id) {mutableStateOf(false)}
    var recenter by remember {mutableIntStateOf(0)}
    var now by remember {mutableLongStateOf((MapDate.now()/1000).toLong())}
    val visible=LocalMotionVisible.current
    LaunchedEffect(part.until,part.stopped,visible) {
        if(visible)do {now=(MapDate.now()/1000).toLong();delay(1000)}while(isActive && part.locationMode=="live" && !part.stopped && now<(part.until ?: now))
    }
    val active=part.locationMode=="live" && !part.stopped && part.until?.let {now<it}==true
    val fresh=active && now>=part.sampledAt && now-part.sampledAt<=60
    val remaining=locationRemaining(part.until,now,part.stopped)
    Box(Modifier.width(280.dp).height(160.dp).clip(RoundedCornerShape(20.dp))) {
        if(!expanded) WebMap(part.latitude,part.longitude,Modifier.fillMaxSize(),avatar=if(part.locationMode!="pin"){{LocationAvatar(name,message.author,fresh)}}else null,controls=false,snapshot=true,placeMarker=part.locationMode=="pin")
        Box(Modifier.matchParentSize().clickable(onClickLabel="Open shared location"){expanded=true})
        if(part.locationMode=="live")LocationMapChip(remaining,Modifier.align(Alignment.TopStart).padding(10.dp))
    }
    if(expanded)androidx.compose.ui.window.Dialog({expanded=false},androidx.compose.ui.window.DialogProperties(usePlatformDefaultWidth=false)) {
        Box(Modifier.widthIn(max=1000.dp).fillMaxSize().padding(12.dp)) {
            WebMap(part.latitude,part.longitude,Modifier.fillMaxSize(),avatar=if(part.locationMode!="pin"){{LocationAvatar(name,message.author,fresh)}}else null,recenter=recenter,placeMarker=part.locationMode=="pin")
            Surface(Modifier.align(Alignment.TopCenter).padding(10.dp).fillMaxWidth(),shape=RoundedCornerShape(24.dp),color=MaterialTheme.colorScheme.surface.copy(alpha=.94f),contentColor=MaterialTheme.colorScheme.onSurface) {
                Row(Modifier.padding(4.dp),verticalAlignment=Alignment.CenterVertically) {
                    Symbol("close","Close map",{expanded=false})
                    Column(Modifier.weight(1f)) {Text(name,style=MaterialTheme.typography.titleMedium);if(part.locationMode=="live")Text(remaining,style=MaterialTheme.typography.labelMedium)}
                    Symbol("my_location","Recenter map",{recenter++})
                }
            }
            if(part.canStop && active && command!=null) Surface(Modifier.align(Alignment.BottomCenter).padding(bottom=36.dp),shape=RoundedCornerShape(24.dp),color=MaterialTheme.colorScheme.surface.copy(alpha=.94f),contentColor=MaterialTheme.colorScheme.onSurface) {
                SigilTextButton({command("location_stop",mapOf("peer" to message.peer,"author" to message.author,"message" to message.id,"card" to part.id))}){Text("Stop sharing")}
            }
        }
    }
}
