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
}
internal val LocalMaterialPress=staticCompositionLocalOf<(() -> Unit)?> {null}
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
    val poseKey="${LocalTextMotion.current?.message}/${value.hashCode()}"
    val key=remember {Any()}
    val anchor=remember(value,outgoing,clock) {MaterialAnchor(value,outgoing,{current.value()},{clock?.generation ?: 0},{clock?.elapsed=12000f},{duration->
        if(clock!=null && duration>0)clock.materialDuration=maxOf(clock.materialDuration,duration.coerceAtMost(12000))
        clock?.preparing?.remove(key);Unit
    })}
    SideEffect {anchor.press=press;anchor.inline=true;anchor.retainPose={poses->
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
