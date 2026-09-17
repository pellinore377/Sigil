package org.sigil

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.animation.core.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.geometry.*
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.first
import kotlin.math.*

internal val MaterialDiceUnit=54.dp
internal val MaterialCoinUnit=80.dp
internal val MaterialCardWidth=148.dp
internal const val MaterialCardAspect=1.47f
internal val MaterialCardHeight=MaterialCardWidth*MaterialCardAspect
internal const val MaterialObjectScale=1.89f
internal val LocalMaterialOpacity=staticCompositionLocalOf {1f}

interface MaterialPlatform {
    val available:Boolean
    // progress >= 1f means settled: the platform view may be released back to the pool.
    @Composable fun Object(kind:Int,sides:Int,face:Int,rotation:FloatArray?,label:String?,modifier:Modifier,progress:Float)
    suspend fun record(data:FloatArray):FloatArray?
    fun horizontalExtent(sides:Int,face:Int,rotation:FloatArray?,outgoing:Boolean):Float
}
val LocalMaterialPlatform=staticCompositionLocalOf<MaterialPlatform> {error("Material platform unavailable")}
@Composable private fun MaterialObject(kind:Int,sides:Int,face:Int,rotation:FloatArray?,label:String?,modifier:Modifier,progress:Float=1f) {
    LocalMaterialPlatform.current.Object(kind,sides,face,rotation,label,modifier.materialOcclusion(),progress)
}
private fun objectTargets(rect:Rect,viewport:Rect,count:Int,unit:Float,outgoing:Boolean):List<Offset> {
    val columns=minOf(3,count);val rows=(count+columns-1)/columns
    return (0 until count).map {i->
        val rowCount=minOf(columns,count-i/columns*columns)
        val x=if(outgoing)rect.right-unit*.85f-(rowCount-1-i%columns)*unit*1.85f else rect.left+unit*.85f+i%columns*unit*1.85f
        Offset(x-viewport.left,rect.top-viewport.top+rect.height/rows*(i/columns+.5f))
    }
}
internal fun previewObjectOrigins(viewport:Rect,origin:Rect,count:Int,size:Float,gap:Float):List<Offset> {
    val step=size+gap
    val available=minOf(viewport.width,origin.width.takeIf {it>=size} ?: viewport.width)
    val columns=minOf(count,maxOf(1,((available+gap+1)/step).toInt()))
    val rows=(count+columns-1)/columns
    val height=rows*step-gap
    val top=(origin.center.y-viewport.top-height/2).coerceIn(0f,(viewport.height-height).coerceAtLeast(0f))
    return (0 until count).map {i->
        val rowCount=minOf(columns,count-i/columns*columns)
        val width=rowCount*step-gap
        val left=(origin.center.x-viewport.left-width/2).coerceIn(0f,(viewport.width-width).coerceAtLeast(0f))
        Offset(left+size/2+(i%columns)*step,top+size/2+(i/columns)*step)
    }
}
private fun ease(value:Float):Float {val p=value.coerceIn(0f,1f);return p*p*p*(p*(p*6-15)+10)}
@OptIn(kotlinx.coroutines.FlowPreview::class)
private suspend fun awaitLayout(anchor:MaterialAnchor,timeline:MaterialTimeline):Boolean = withTimeoutOrNull(2000) {
    snapshotFlow {Triple(anchor.bounds,timeline.viewport,timeline.bubbles.toMap())}.debounce(120).first {it.first.width>0 && it.second.height>0}
    true
} ?: false
@Composable internal fun PickerCards(value:RandomizerMotion,progress:Float,modifier:Modifier,targetX:Float?=null,centerY:Float?=null,press:(()->Unit)?=null,launch:Offset?=null,departure:(()->Unit)?=null,targetY:Float?=null) {
    BoxWithConstraints(modifier) {
        val inline=LocalMaterialInline.current;val outgoing=LocalMaterialOutgoing.current
        val width=constraints.maxWidth.toFloat();val height=constraints.maxHeight.toFloat();val density=LocalDensity.current
        val haptic=LocalHapticFeedback.current
        val cardWidth=minOf(width*(if(LocalObjectMenu.current || LocalMaterialInline.current).9f else .46f),with(density){MaterialCardWidth.toPx()});val cardHeight=cardWidth*MaterialCardAspect
        val cy=centerY ?: height/2
        val raw=progress.coerceIn(0f,1f)
        val p=if(launch!=null)((raw-.22f)/.78f).coerceIn(0f,1f)else raw
        val travel=(1-ease((p-.10f)/.45f))*14f
        val lift=ease((p-.54f)/.1f);val flip=if(LocalCardBack.current)0f else ease((p-.65f)/.15f);val fold=ease((p-.61f)/.2f)
        val extent=minOf((width-cardWidth)/2-4,cardWidth*1.65f).coerceAtLeast(0f);val step=extent/3
        val visible=if(p>=.83f || launch!=null && p<.025f)listOf(3)else(0..6).toList()
        val mistColor=Color(LocalAppearance.current.cardStyle.second or 0xff000000.toInt())
        for(front in listOf(false,true))Canvas(Modifier.matchParentSize().zIndex(if(front)11f else -1f).testTag(if(front)"card-mist-front" else "card-mist-back")) {
            val life=(1-(1-(p/.13f).coerceIn(0f,1f)).pow(3))*(1-ease((p-.75f)/.17f))
            if(life>0f)repeat(if(front)23 else 50) {i->
                fun noise(n:Int):Float {
                    var h=i*7919+n*104729+37
                    h=(h xor(h ushr 16))*0x45d9f3b;h=(h xor(h ushr 16))*0x45d9f3b
                    return ((h xor(h ushr 16))and 0xffffff)/16777215f
                }
                val phase=noise(1)*2*PI.toFloat();val r=noise(2)
                val x=width/2+(if(i%2==0)-1 else 1)*(extent*(.78f+noise(3)*.36f)+sin(p*9*(.5f+noise(4))+phase)*cardWidth*.08f)
                val y=cy+(noise(5)-.42f)*cardHeight*.91f+sin(p*6+phase)*cardHeight*.06f
                if(i<26) {
                    val radius=cardWidth*(.16f+r*.33f)
                    val tint=lerp(mistColor,Color.White,.45f).copy(alpha=(if(front).075f else .065f)*life*(.5f+r*.5f))
                    scale(1f,.66f,Offset(x,y)) {drawCircle(Brush.radialGradient(listOf(tint,tint.copy(alpha=tint.alpha*.55f),Color.Transparent),Offset(x,y),radius),radius,Offset(x,y))}
                } else drawCircle(lerp(mistColor,Color.White,.7f).copy(alpha=life*(.16f+.23f*r)*(.5f+.5f*sin(p*13+phase).pow(2))),cardWidth*(.003f+r*.005f),Offset(x,y))
            }
        }
        for(i in visible)key(i) {
            val chosen=i==3;val slot=(((i-3+travel+3.5f)%7)+7)%7-3.5f;val distance=abs(slot)
            val expanded=sign(slot)*(step*distance+maxOf(step*.3f,(cardWidth*1.16f-step)/sqrt(2f))*sin(minOf(distance,1f)*PI.toFloat()/2))
            val focus=exp(-(slot/.52f).pow(4));val spread=if(launch!=null)ease(p/.12f)else 1f
            var x=width/2+expanded*spread;var y=cy+((slot/3).pow(2)*cardHeight*.135f-focus*cardHeight*.065f)*spread+if(launch==null)(1-ease(p/.15f))*cardHeight*.5f else 0f
            var angle=slot/3*23;var opacity=ease(p/.09f)*(1-ease((distance-2.55f)/.65f))
            if(distance>1.1f)opacity*=1-ease((abs(expanded)-extent)/(cardWidth*.4f))
            if(chosen&&p>.54f) {
                val dock=if(launch!=null)ease((raw-.8f)/.2f)else ease((p-.85f)/.13f)
                x=width/2+((targetX ?: if(inline)if(outgoing)width-cardWidth*.395f else cardWidth*.395f else width/2)-width/2)*dock
                y=cy+((targetY ?: cy)-cy)*dock-cardHeight*(.065f+.055f*lift)*(1-dock)
                angle*=1-lift;opacity=1f
            }
            else if(p>.56f) {y+=fold*32;opacity*=1-fold}
            launch?.let { start ->
                val departure=ease(raw/.22f)
                x=start.x+(x-start.x)*departure
                y=start.y+(y-start.y)*departure
                angle*=departure*spread
                if(chosen && p<.025f)opacity=1f
                if(!chosen)opacity*=ease(p/.12f)
            }
            val flipAngle=if(chosen)PI.toFloat()*(1-flip)/2 else PI.toFloat()/2
            val tilt=-angle*PI.toFloat()/360
            val q=floatArrayOf(-sin(tilt)*sin(flipAngle),cos(tilt)*sin(flipAngle),sin(tilt)*cos(flipAngle),cos(tilt)*cos(flipAngle))
            val sizeScale=1-.045f*distance.coerceAtMost(3f)
            val objectWidth=cardWidth*sizeScale;val objectHeight=cardHeight*sizeScale
            CompositionLocalProvider(LocalMaterialOpacity provides opacity) {
                MaterialObject(2,0,0,q,if(chosen)value.result else "",Modifier.offset {IntOffset((x-objectWidth/2).roundToInt(),(y-objectHeight/2).roundToInt())}.size(with(density){objectWidth.toDp()},with(density){objectHeight.toDp()}).graphicsLayer {alpha=opacity}.testTag("picker-card-$i").then(if(press!=null)Modifier.pointerInput(press){detectTapGestures(onLongPress={haptic.performHapticFeedback(HapticFeedbackType.LongPress);press()})}.semantics{onLongClick("Message actions"){press();true}}else Modifier).zIndex(if(chosen&&p>.54f)10f else 4-distance),progress=raw)
            }
        }
        SideEffect {departure?.invoke()}
    }
}
data class Flight(val data:FloatArray,val count:Int,val unit:Float,val viewport:Rect,val anchor:Rect,val fromPreview:Boolean=false) {
    val frames=(data.size-1)/(count*7)
    val duration=((frames-1)*1000f/60).roundToInt()
    fun pose(index:Int,p:Float):FloatArray {
        val t=(p*(frames-1)).coerceIn(0f,(frames-1).toFloat())
        val a=t.toInt();val b=minOf(a+1,frames-1);val f=t-a
        // Adjacent recorded orientations differ by much less than a half-turn.
        val dot=(3..6).sumOf {j->data[1+(a*count+index)*7+j].toDouble()*data[1+(b*count+index)*7+j]}
        val out=FloatArray(7) {j->val x=data[1+(a*count+index)*7+j];val y=data[1+(b*count+index)*7+j]*(if(j>=3&&dot<0)-1f else 1f);x+(y-x)*f}
        val length=sqrt((3..6).sumOf {out[it].toDouble()*out[it]}.toFloat());for(j in 3..6)out[j]/=length
        return out
    }
}
@Composable fun MaterialTimelineOverlay(timeline:MaterialTimeline,modifier:Modifier) {
    val viewport=timeline.viewport
    if(viewport.width<1||viewport.height<1)return
    val handoff=LocalMaterialHandoffPass.current
    val anchors=timeline.anchors.values.filter {it.bounds.overlaps(viewport) && (!it.inline || it.progress()<1f) && (it.launchOrigin!=null && it.progress()<1f)==handoff}.take(12)
    CompositionLocalProvider(LocalMaterialTimeline provides timeline) {Box(modifier) {
        for(anchor in anchors)key(anchor,anchor.generation()) {
            DisposableEffect(anchor) {onDispose {if(handoff)anchor.launchHost?.departed(anchor.message,anchor.ordinal)}}
            CompositionLocalProvider(LocalMaterialLaunchWindow provides anchor.launchOrigin.takeIf {anchor.value.kind=="choice" || anchor.launchHost?.isActive(anchor.message)==true}) {
            val p=anchor.progress()
            if(anchor.value.kind=="choice") {
                var origin by remember {mutableStateOf<Rect?>(null)}
                LaunchedEffect(anchor) {
                    if(p<1f && anchor.launchOrigin==null && !awaitLayout(anchor,timeline))anchor.settle()
                    origin=anchor.bounds;anchor.ready(0)
                }
                val rect=anchor.bounds;val density=LocalDensity.current
                val cardWidth=minOf(viewport.width*.46f,with(density){MaterialCardWidth.toPx()})
                val visibleHalfWidth=cardWidth*.395f
                val target=if(anchor.outgoing)rect.right-viewport.left-visibleHalfWidth else rect.left-viewport.left+visibleHalfWidth
                val center=(anchor.launchOrigin?.center?.y ?: rect.center.y)-viewport.top
                if(origin!=null)PickerCards(anchor.value,p,Modifier.matchParentSize(),target,center,anchor.press.takeUnless {handoff},anchor.launchOrigin?.center?.minus(viewport.topLeft),{
                    anchor.launchHost?.started(anchor.message,anchor.ordinal)
                    anchor.launchHost?.departed(anchor.message,anchor.ordinal)
                },rect.center.y-viewport.top)
            } else FlightObjects(anchor,timeline,viewport,p)
            }
        }
    }}
}
@Composable private fun FlightObjects(anchor:MaterialAnchor,timeline:MaterialTimeline,viewport:Rect,progress:Float) {
    val platform=LocalMaterialPlatform.current
    val interactive=!LocalMaterialHandoffPass.current
    val haptic=LocalHapticFeedback.current
    val value=anchor.value;val coin=value.kind=="coin";val items=if(coin)listOf(0 to value.selected)else value.dice.take(6).map {it.sides to it.face}
    if(items.isEmpty())return
    val density=LocalDensity.current;val rect=anchor.bounds
    val columns=minOf(3,items.size);val rows=(items.size+columns-1)/columns
    val unit=minOf(with(density){if(coin)MaterialCoinUnit.toPx()else MaterialDiceUnit.toPx()},rect.width/(columns*1.85f),rect.height/(rows*2.1f))
    if(unit<1)return
    val targets=objectTargets(rect,viewport,items.size,unit,anchor.outgoing)
    var flight by remember(anchor) {mutableStateOf<Flight?>(null)}
    var attempted by remember(anchor) {mutableStateOf(false)}
    // Snapshot geometry once per message presentation; scrolling never rerolls or restarts it.
    LaunchedEffect(anchor) {
        if(progress>=1f || !platform.available) {attempted=true;anchor.ready(0);return@LaunchedEffect}
        if(anchor.launchOrigin==null && !awaitLayout(anchor,timeline)) {attempted=true;anchor.settle();anchor.ready(0);return@LaunchedEffect}
        val viewport=timeline.viewport;val rect=anchor.bounds
        val targets=objectTargets(rect,viewport,items.size,unit,anchor.outgoing)
        val starts=anchor.launchOrigin?.let {previewObjectOrigins(viewport,it,items.size,unit*MaterialObjectScale,with(density){4.dp.toPx()})}
        val obstacles=timeline.bubbles.values.filter {r->r.overlaps(viewport)&&!r.contains(anchor.bounds.center)}.take(64).map {r->Rect(maxOf(r.left,viewport.left),maxOf(r.top,viewport.top),minOf(r.right,viewport.right),minOf(r.bottom,viewport.bottom))}
        val input=buildList<Float> {
            add(viewport.width/unit);add(viewport.height/unit);add(items.size.toFloat());add(obstacles.size.toFloat());add(if(anchor.outgoing)1f else 0f);add((value.hashCode()and 0x7fffff).toFloat())
            items.forEachIndexed {i,(sides,face)->add(sides.toFloat());add(face.toFloat());add(targets[i].x/unit);add(targets[i].y/unit)}
            for(r in obstacles){add((r.left-viewport.left)/unit);add((r.top-viewport.top)/unit);add((r.right-viewport.left)/unit);add((r.bottom-viewport.top)/unit)}
            starts?.forEach {add(it.x/unit);add(it.y/unit)}
        }.toFloatArray()
        val recorded=withContext(Dispatchers.Default){platform.record(input)}
        if(recorded!=null&&recorded.size>=1+items.size*7) {
            val plan=Flight(recorded,items.size,unit,viewport,rect,starts!=null)
            if(plan.duration<=12000) {flight=plan;anchor.retainPose?.invoke(items.indices.map {plan.pose(it,1f).copyOfRange(3,7)})}
        }
        attempted=true
        anchor.ready(flight?.duration ?: 0)
    }
    var returning by remember(anchor) {mutableStateOf<List<FloatArray>?>(null)}
    val motion=LocalMotion.current
    val dock=remember(anchor){Animatable(0f)}
    val geometryStable=flight?.let {abs(it.viewport.width-viewport.width)<2&&abs(it.viewport.height-viewport.height)<2} ?: true
    LaunchedEffect(flight) {
        val plan=flight
        if(plan!=null) {
            snapshotFlow {timeline.viewport}.first {v->abs(plan.viewport.width-v.width)>=2||abs(plan.viewport.height-v.height)>=2}
            returning=items.indices.map {plan.pose(it,anchor.progress())}
            dock.animateTo(1f,motion.tween(MotionSettle))
            anchor.settle()
        }
    }
    if(!attempted)return
    var allDeparted=true
    for(i in items.indices)key(i) {
        val plan=flight
        val pose=if(plan!=null)plan.pose(i,progress)else null
        val source=returning?.get(i)
        val extent=remember(plan,items[i],anchor.outgoing) {
            if(platform.available)platform.horizontalExtent(items[i].first,items[i].second,plan?.pose(i,1f)?.copyOfRange(3,7),anchor.outgoing)else 1.14f
        }
        val insetCorrection=(.85f-extent*.7f)*unit*(if(anchor.outgoing)1f else -1f)
        val returnStart=plan?.let {it.data[0]/(it.frames-1)} ?: 0f
        val alignment=if(plan==null||progress>=1f)1f else ease((progress-returnStart)/(1f-returnStart).coerceAtLeast(.001f))
        val relocation=if(plan?.fromPreview==true)alignment else 1f
        val x=insetCorrection*alignment+if(source!=null)source[0]*plan!!.unit+(targets[i].x-source[0]*plan.unit)*dock.value else if(pose!=null && progress<1f)pose[0]*plan!!.unit+((rect.left-viewport.left)-(plan.anchor.left-plan.viewport.left))*relocation else targets[i].x
        val y=if(source!=null)source[2]*plan!!.unit+(targets[i].y-source[2]*plan.unit)*dock.value else if(pose!=null && progress<1f)pose[2]*plan!!.unit+((rect.top-viewport.top)-(plan.anchor.top-plan.viewport.top))*relocation else targets[i].y
        val q=if(source!=null) {
            val end=plan!!.pose(i,1f)
            val sign=if((3..6).sumOf {source[it].toDouble()*end[it]}<0)-1f else 1f
            FloatArray(4) {j->source[j+3]+(end[j+3]*sign-source[j+3])*dock.value}.also {r->
                val length=sqrt(r.sumOf {it.toDouble()*it}.toFloat());for(j in r.indices)r[j]/=length
            }
        } else pose?.copyOfRange(3,7)?.let {rotation->
            if(anchor.launchOrigin==null || progress>=.14f)rotation else {
                val fraction=ease(progress/.14f)
                val sign=if(rotation[3]<0)-1f else 1f
                FloatArray(4){j->rotation[j]*sign*fraction+(if(j==3)1-fraction else 0f)}.also {q->
                    val length=sqrt(q.sumOf {it.toDouble()*it}.toFloat());for(j in q.indices)q[j]/=length
                }
            }
        }
        val size=(plan?.unit ?: unit)*MaterialObjectScale
        val altitude=if(pose!=null && plan!=null && geometryStable)maxOf(0f,pose[1]-plan.pose(i,1f)[1])else 0f
        val lift=altitude.coerceAtMost(2.5f)*unit*.42f
        val scale=1+(altitude*.08f).coerceAtMost(.2f)
        val clearance=LocalMaterialOcclusion.current?.footer?.top ?: anchor.launchOrigin?.top ?: viewport.bottom
        if(y-lift+size*scale/2>clearance-viewport.top)allDeparted=false
        val window=LocalMaterialLaunchWindow.current?.let {origin->
            Rect(minOf(origin.left,viewport.left+x-size*scale/2),minOf(origin.top,viewport.top+y-lift-size*scale/2),maxOf(origin.right,viewport.left+x+size*scale/2),maxOf(origin.bottom,viewport.top+y-lift+size*scale/2))
        }
        Box(Modifier.testTag(if(plan!=null)"material-flight-recorded" else "material-flight-still").offset {IntOffset((x-size/2).roundToInt(),(y-size/2).roundToInt())}.size(with(density){size.toDp()})) {
            Canvas(Modifier.fillMaxSize()) {
                val radius=this.size.minDimension*(.38f+altitude*.035f)
                drawCircle(Brush.radialGradient(listOf(Color.Black.copy(alpha=.15f/(1+altitude)),Color.Transparent),center=this.center,radius=radius),radius,this.center)
            }
        }
        CompositionLocalProvider(LocalMaterialLaunchWindow provides window) {
            MaterialObject(if(coin)1 else 0,items[i].first,items[i].second,q,if(coin)value.result else value.dice.getOrNull(i)?.marking?.takeIf {it.isNotEmpty()},Modifier.offset {IntOffset((x-size*scale/2).roundToInt(),(y-lift-size*scale/2).roundToInt())}.size(with(density){(size*scale).toDp()}).testTag("material-object-$i").then(if(interactive)Modifier.pointerInput(anchor){detectTapGestures(onLongPress={haptic.performHapticFeedback(HapticFeedbackType.LongPress);anchor.press?.invoke()})}.semantics{onLongClick("Message actions"){anchor.press?.invoke();true}}else Modifier),if(progress<1f || source!=null && dock.value<1f).5f else 1f)
        }
    }
    SideEffect {
        anchor.launchHost?.started(anchor.message,anchor.ordinal)
        if(allDeparted || flight==null || progress>=1f)anchor.launchHost?.departed(anchor.message,anchor.ordinal)
    }
}
@Composable fun MaterialMessages(value:RandomizerMotion,progress:Float,modifier:Modifier) {
    val platform=LocalMaterialPlatform.current
    if(value.kind=="choice") {PickerCards(value,progress,modifier);return}
    val coin=value.kind=="coin"
    val items=if(coin)listOf(0 to value.selected)else value.dice.take(6).map {it.sides to it.face}
    val columns=minOf(3,items.size).coerceAtLeast(1)
    BoxWithConstraints(modifier) {
        val rows=(items.size+columns-1)/columns
        val unit=minOf(if(coin)MaterialCoinUnit else MaterialDiceUnit,maxWidth/(columns*1.85f),if(LocalObjectMenu.current)100.dp else maxHeight/(rows.coerceAtLeast(1)*2.1f))
        val inline=LocalMaterialInline.current;val outgoing=LocalMaterialOutgoing.current;val poses=LocalMaterialRestPoses.current
        items.forEachIndexed {i,(sides,face)->
            val q=poses.getOrNull(i)
            val extent=remember(sides,face,q,outgoing) {if(platform.available)platform.horizontalExtent(sides,face,q,outgoing)else 1.14f}
            val rowCount=minOf(columns,items.size-i/columns*columns)
            val x=if(inline)if(outgoing)maxWidth-unit*(extent*.7f+(rowCount-1-i%columns)*1.85f) else unit*(extent*.7f+i%columns*1.85f) else maxWidth/columns*(i%columns+.5f)
            Box(Modifier.offset(x=x-unit*.945f,y=maxHeight/rows*(i/columns+.5f)-unit*.945f).size(unit*MaterialObjectScale)) {
                MaterialObject(if(coin)1 else 0,sides,face,q,if(coin)value.result else value.dice.getOrNull(i)?.marking?.takeIf {it.isNotEmpty()},Modifier.fillMaxSize().testTag("material-object-$i"),progress)
            }
        }
    }
}
