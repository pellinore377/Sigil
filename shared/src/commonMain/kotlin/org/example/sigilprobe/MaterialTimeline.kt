package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.layout.onGloballyPositioned

class MaterialTimeline {
    var viewport by mutableStateOf(Rect.Zero)
    val restPoses=mutableStateMapOf<String,List<FloatArray>>()
    val bubbles=mutableStateMapOf<String,Rect>()
    val anchors=mutableStateMapOf<Any,MaterialAnchor>()
}
class MaterialAnchor(val value:RandomizerMotion,val outgoing:Boolean,val progress:()->Float,val generation:()->Int,val settle:()->Unit={},val ready:(Int)->Unit={}) {
    var inline=false
    var retainPose:((List<FloatArray>)->Unit)?=null
    var bounds by mutableStateOf(Rect.Zero)
    var press:(()->Unit)?=null
    var launchOrigin by mutableStateOf<Rect?>(null)
    internal var launchHost:PreviewLaunch?=null
    var message:String?=null
    var ordinal=0
}
internal class PreviewLaunch {
    var source = ""
    var bounds = Rect.Zero
    var panel = Rect.Zero
    val visibleOrigins = mutableMapOf<Int, Rect>()
    private var pending: Triple<String, Map<Int,Rect>, Long>? = null
    private var pendingPanel: Triple<String, Rect, Long>? = null
    private val origins = mutableStateMapOf<String, Map<Int,Rect>>()
    private val panelOrigins = mutableStateMapOf<String, Rect>()
    var activeSource by mutableStateOf<String?>(null)
        private set
    var activeMessage by mutableStateOf<String?>(null)
        private set
    private val lifted=mutableStateMapOf<Int,Boolean>()
    private val departed=mutableStateMapOf<Int,Boolean>()
    fun holding(value:String?)=value!=null && activeSource==value
    fun isActive(message:String?)=message!=null && activeMessage==message && activeSource!=null
    fun lifted(ordinal:Int)=lifted[ordinal]==true
    fun arm(value: String, sent: Long) {
        if(activeSource!=null)return
        pending = bounds.takeIf { value == source && it.width > 0 && it.height > 0 }?.let { Triple(value, visibleOrigins.toMap().ifEmpty {mapOf(0 to it)}, sent) }
        if(pending!=null){activeSource=value;activeMessage=null;lifted.clear();departed.clear()}
    }
    fun bind(value: String, message: String, sent: Long) {
        val flight = pending ?: return
        if (flight.first != value || sent <= flight.third) return
        if (origins.size >= 16) origins.remove(origins.keys.first())
        origins[message] = flight.second
        activeMessage=message
        pending = null
    }
    // Cards and bubbles fly from the preview panel itself; objects already carry their own per-object origins.
    fun armPanel(value: String, sent: Long) {
        if (activeSource != null) return
        pendingPanel = panel.takeIf { value == source && it.width > 0 && it.height > 0 }?.let { Triple(value, it, sent) }
    }
    fun bindPanel(value: String, message: String, sent: Long) {
        val flight = pendingPanel ?: return
        if (flight.first != value || sent <= flight.third) return
        if (panelOrigins.size >= 16) panelOrigins.remove(panelOrigins.keys.first())
        panelOrigins[message] = flight.second
        pendingPanel = null
    }
    fun panelOrigin(message: String) = panelOrigins[message]
    fun landed(message: String) { panelOrigins.remove(message) }
    fun origin(message: String, ordinal:Int=0) = origins[message]?.get(ordinal)
    fun started(message:String?,ordinal:Int) {if(isActive(message))lifted[ordinal]=true}
    fun departed(message:String?,ordinal:Int) {
        if(!isActive(message))return
        departed[ordinal]=true
        if(origins[message]?.keys?.all {departed[it]==true}==true){activeSource=null;activeMessage=null}
    }
    fun cancel() { pending = null;pendingPanel = null;activeSource=null;activeMessage=null;lifted.clear();departed.clear() }
}
internal val LocalMaterialHandoffPass=staticCompositionLocalOf {false}
internal val LocalPreviewLaunch = staticCompositionLocalOf<PreviewLaunch?> { null }
internal val LocalMaterialOrdinal = staticCompositionLocalOf {0}
val LocalMaterialPress=staticCompositionLocalOf<(() -> Unit)?> {null}
val LocalMaterialOverlay=staticCompositionLocalOf<(@Composable (MaterialTimeline,Modifier)->Unit)?> {null}
internal val LocalMaterialTimeline=staticCompositionLocalOf<MaterialTimeline?> {null}
val LocalMaterialOutgoing=staticCompositionLocalOf {false}
val LocalMaterialRestPoses=staticCompositionLocalOf<List<FloatArray>> {emptyList()}
val LocalMaterialInline=staticCompositionLocalOf {false}
val LocalObjectMenu=staticCompositionLocalOf {false}
@Composable internal fun MaterialSlot(value:RandomizerMotion,progress:()->Float,modifier:Modifier) {
    val render=LocalSolidMaterial.current ?: return
    val timeline=LocalMaterialTimeline.current
    if(timeline==null) {render(value,progress(),modifier);return}
    val outgoing=LocalMaterialOutgoing.current
    val press=LocalMaterialPress.current
    val current=rememberUpdatedState(progress)
    val clock=LocalTextMotion.current?.clock
    val launch = LocalPreviewLaunch.current
    val message = LocalTextMotion.current?.message
    val ordinal = LocalMaterialOrdinal.current
    val poseKey="${LocalTextMotion.current?.message}/${value.hashCode()}"
    val key=remember {Any()}
    val anchor=remember(value,outgoing,clock) {MaterialAnchor(value,outgoing,{current.value()},{clock?.generation ?: 0},{clock?.elapsed=12000f},{duration->
        if(clock!=null && duration>0)clock.materialDuration=maxOf(clock.materialDuration,duration.coerceAtMost(12000))
        clock?.preparing?.remove(key);Unit
    })}
    SideEffect {anchor.launchHost=launch;anchor.message=message;anchor.ordinal=ordinal;anchor.launchOrigin=if (outgoing && clock?.generation == 0 && message != null) launch?.origin(message,ordinal) else null;anchor.press=press;anchor.inline=true;anchor.retainPose={poses->
        if(timeline.restPoses.size>=256 && poseKey !in timeline.restPoses)timeline.restPoses.remove(timeline.restPoses.keys.first())
        timeline.restPoses[poseKey]=poses
    }}
    DisposableEffect(anchor,clock?.generation) {
        if(progress()<1f)clock?.preparing?.put(key,Unit)
        onDispose {clock?.preparing?.remove(key)}
    }
    DisposableEffect(timeline,anchor) {timeline.anchors[key]=anchor;onDispose {timeline.anchors.remove(key)}}
    Box(modifier.onGloballyPositioned {anchor.bounds=Rect(it.localToWindow(Offset.Zero),Size(it.size.width.toFloat(),it.size.height.toFloat()))}) {
        if(progress()>=1f)CompositionLocalProvider(LocalMaterialRestPoses provides timeline.restPoses[poseKey].orEmpty(),LocalMaterialInline provides true) {render(value,1f,Modifier.matchParentSize())}
    }
}
