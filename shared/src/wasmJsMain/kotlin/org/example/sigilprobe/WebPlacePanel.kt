@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.unit.dp
import androidx.compose.ui.draw.clip
import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import kotlin.math.roundToInt
import kotlin.js.*
@JsName("Date") private external class LocationDate:JsAny {companion object {fun now():Double}}

internal class WebLocations {
    var point by mutableStateOf<JsonObject?>(null)
    var issue by mutableStateOf<String?>(null)
    var locating by mutableStateOf(false)
        private set
    var live by mutableStateOf(false)
    var completion by mutableIntStateOf(0)
        private set
    fun shared(mode:String,foreground:Boolean) {
        if(!foreground)live=false else if(mode=="live")live=true
        completion++
        if(!live)stop()
    }
    private var watch:BrowserLocationWatch?=null
    fun locate() {
        if(watch!=null)return
        point=null;issue=null;locating=true
        try {watch=BrowserLocationWatch({source->point=Json.parseToJsonElement(source).jsonObject;issue=null;locating=false},{text->issue=text;locating=false})}
        catch(_:Exception){issue="Allow location access in your browser, then retry.";locating=false}
    }
    fun stop() {watch?.free();watch=null;point=null;locating=false}
    fun fresh():JsonObject?=point?.takeIf {val age=(LocationDate.now()/1000).toLong()-(it["sampled_at"]?.jsonPrimitive?.longOrNull ?: 0);age in 0..60}
}

@Composable internal fun WebPlacePanel(target:Map<String,Any?>,locations:WebLocations,back:()->Unit,photo:String="",share:suspend(Map<String,Any?>)->Unit) {
    val caption=target["location_caption"] as? String ?: ""
    val locating=locations.locating
    val mode=target["location_mode"] as? String ?: "once"
    var centerOnDevice by remember {mutableStateOf(false)}
    var pinPoint by remember {mutableStateOf<Pair<Double,Double>?>(null)}
    var duration by remember {mutableStateOf("fifteen_minutes")}
    var sending by remember {mutableStateOf(false)}
    var issue by remember {mutableStateOf<String?>(null)}
    val scope=rememberCoroutineScope()
    DisposableEffect(locations){onDispose {if(!locations.live)locations.stop()}}
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
      Column(Modifier.fillMaxWidth().then(naturalPanelHeight()).padding(8.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        val coords=locations.point?.get("coordinates")?.jsonObject
        val mapLat=pinPoint?.first ?: coords?.get("latitude_e6")?.jsonPrimitive?.int?.div(1_000_000.0) ?: 39.0
        val mapLon=pinPoint?.second ?: coords?.get("longitude_e6")?.jsonPrimitive?.int?.div(1_000_000.0) ?: -98.0
        val devicePoint=coords?.let {it["latitude_e6"]!!.jsonPrimitive.int/1_000_000.0 to it["longitude_e6"]!!.jsonPrimitive.int/1_000_000.0}
        Box(Modifier.fillMaxWidth().height(200.dp).clip(RoundedCornerShape(20.dp))) {
            WebMap(mapLat,mapLon,Modifier.fillMaxSize(),showMarker=if(mode=="pin")pinPoint!=null else coords!=null,
                pin=if(mode=="pin") {a,b->pinPoint=a to b;centerOnDevice=false} else null,center=devicePoint.takeIf {centerOnDevice},focused=pinPoint!=null || coords!=null,avatar=if(mode!="pin"){{LocationAvatar("You",photo,false)}}else null)
            Surface(Modifier.align(Alignment.TopStart).padding(4.dp),shape=RoundedCornerShape(24.dp),color=MaterialTheme.colorScheme.surface.copy(alpha=.94f),contentColor=MaterialTheme.colorScheme.onSurface) {
                Symbol("chevron_left","Back to attachments",back)
            }
            LocationMapChip(when(mode){"pin"->"Drop a pin";"live"->"Real-time location";else->"One-time location"},Modifier.align(Alignment.TopStart).padding(start=60.dp,end=60.dp,top=12.dp))
            Surface(Modifier.align(Alignment.TopEnd).padding(8.dp),shape=RoundedCornerShape(24.dp),color=MaterialTheme.colorScheme.surface.copy(alpha=.94f),contentColor=MaterialTheme.colorScheme.onSurface) {
                SigilIconButton({issue=null;centerOnDevice=mode=="pin";locations.stop();locations.locate()},enabled=!locating && !sending) {
                    if(locating)CircularProgressIndicator(Modifier.size(20.dp),strokeWidth=2.dp)
                    else Glyph("my_location",24,if(mode=="pin" || locations.point==null)"Use my location" else "Refresh location")
                }
            }
        }
        if(mode=="live") {
            FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                listOf("fifteen_minutes" to "15 min","hour" to "1 hour","eight_hours" to "8 hours").forEach {(id,label)->FilterChip(duration==id,{duration=id},label={Text(label)},shape=RoundedCornerShape(14.dp))}
            }
            Text("Keep this tab open to share live.",style=MaterialTheme.typography.bodySmall)
        }
        (issue ?: locations.issue)?.let {Text(it,color=MaterialTheme.colorScheme.error,style=MaterialTheme.typography.bodySmall)}
        BuilderConfirm(if(sending)"Preparing…" else if(mode=="live")"Share live location" else "Send place",enabled=!sending && (if(mode=="pin")pinPoint!=null else locations.point!=null)) {
            if(caption.encodeToByteArray().size>256){issue="Shorten the location caption.";return@BuilderConfirm}
            val point=if(mode=="pin") {
                val (lat,lon)=pinPoint ?: return@BuilderConfirm
                mapOf("latitude_e6" to (lat*1_000_000).roundToInt(),"longitude_e6" to (lon*1_000_000).roundToInt(),"sampled_at" to (LocationDate.now()/1000).toLong())
            } else {
                val sample=locations.fresh() ?: run {issue="Refresh your location before sharing.";return@BuilderConfirm}
                val coords=sample["coordinates"]!!.jsonObject
                mapOf("latitude_e6" to coords["latitude_e6"]!!.jsonPrimitive.int,"longitude_e6" to coords["longitude_e6"]!!.jsonPrimitive.int,"accuracy_cm" to sample["accuracy_cm"]!!.jsonPrimitive.long,"sampled_at" to sample["sampled_at"]!!.jsonPrimitive.long)
            }
            sending=true;scope.launch {
                try {share(target.filterKeys {it!="location_mode" && it!="location_caption"}+point+mapOf("label" to caption.ifBlank {if(mode=="pin")"Dropped pin" else "My location"},"pin" to (mode=="pin"),"live" to duration.takeIf {mode=="live"}))}
                catch(cancelled:CancellationException){throw cancelled}
                catch(_:Exception){issue="Could not share this place. Try again."}
                finally{sending=false}
            }
        }
    }
  }
}
