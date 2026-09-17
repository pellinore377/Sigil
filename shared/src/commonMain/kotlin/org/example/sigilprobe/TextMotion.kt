package org.sigil

import androidx.compose.runtime.*
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.*
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.compose.ui.graphics.rememberGraphicsLayer
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.dp
import kotlin.math.*

val LocalTextMotionSeeds=staticCompositionLocalOf<((String)->String)?> {null}
val LocalMotionVisible=staticCompositionLocalOf {true}
val LocalMotionBlur=staticCompositionLocalOf {true}
internal class TextPlayback(fresh:Boolean=false) {
    val preparing=mutableStateMapOf<Any,Unit>()
    var materialDuration by mutableIntStateOf(0)
    fun duration(default:Int)=if(materialDuration>0)maxOf(minOf(default,TextMotionDefault),materialDuration)else default
    var elapsed by mutableFloatStateOf(if(fresh)0f else TextMotionCap.toFloat())
    var generation by mutableIntStateOf(0)
    fun replay() {elapsed=0f;materialDuration=0;generation++}
}
internal class MotionLedger {
    private var initialized=false
    private var previous=emptySet<String>()
    private val entries=linkedMapOf<String,TextPlayback>()
    fun update(ids:List<String>,loaded:Boolean,allowNew:Boolean,animated:Set<String> = ids.toSet()) {
        if(!loaded)return
        val boundary=if(!initialized || !allowNew)0 else ids.indexOfFirst {it in previous}.let {if(it>=0)it else if(previous.isEmpty())ids.size else 0}
        ids.take(boundary).filter {it in animated}.forEach {id->if(id !in entries)put(id,TextPlayback(true))}
        previous=ids.toSet();initialized=true
    }
    private fun put(id:String,value:TextPlayback):TextPlayback {
        entries.remove(id)
        if(entries.size>=512)entries.remove(entries.keys.first())
        entries[id]=value
        return value
    }
    fun state(id:String)=put(id,entries[id] ?: TextPlayback())
}
internal data class TextMotionContext(val message:String,val clock:TextPlayback)
internal val LocalTextMotion=staticCompositionLocalOf<TextMotionContext?> {null}

internal const val ChartMotionMillis=700
internal const val TextMotionCap=12000
internal const val TextMotionDefault=2000
internal fun ChatMessage.messageMotionDuration()=parts.maxOfOrNull {p->
    val texts=buildList {
        p.rich?.let(::add);addAll(p.items.mapNotNull {it.rich})
        p.table?.let {addAll(it.columns);it.rows.forEach(::addAll)}
        p.recipe?.let {add(it.title);addAll(it.ingredients);addAll(it.steps)}
        p.chart?.let {add(it.title);addAll(it.points.map {point->point.label})}
        p.diagram?.let {add(it.title);addAll(it.nodes.map {node->node.label});addAll(it.edges.map {edge->edge.label});it.entries.forEach {entry->add(entry.date);add(entry.label)}}
        p.utility?.let {it.rich?.let(::add);it.secondary?.let(::add);addAll(it.details)}
        p.contact?.let {add(it.name)}
        p.service?.let {s->
            add(s.title);add(s.attribution);s.original?.let(::add);s.pronunciation?.let(::add)
            s.senses.forEach {add(it.part);add(it.definition);it.example?.let(::add);it.etymology?.let(::add);addAll(it.synonyms);addAll(it.antonyms)}
            s.current?.let {add(it.description)};addAll(s.days.map {it.description});addAll(s.hours.map {it.description})
        }
    }
    maxOf(if(p.chart!=null)ChartMotionMillis else 0,p.utility?.motion?.let(::randomizerDuration) ?: 0,texts.maxOfOrNull {t->t.motion.maxOfOrNull {if(it.kind=="typewriter" && it.stagger>0)minOf(it.duration,it.stagger*it.units.size) else it.duration} ?: 0} ?: 0)
} ?: 0
internal fun ChatMessage.hasMessageMotion()=messageMotionDuration()>0

@Composable
internal fun MessageMotion(message:String,clock:TextPlayback,visible:Boolean,duration:Int=TextMotionDefault,content:@Composable ()->Unit) {
    val active=visible && LocalMotionVisible.current
    val reduced=LocalMotion.current.reduced || !LocalAppearance.current.messageEffects
    val replaySeconds=LocalAppearance.current.replaySeconds
    LaunchedEffect(clock,active,reduced,clock.generation,duration,replaySeconds) {
        if(reduced)clock.elapsed=TextMotionCap.toFloat()
        if(active && !reduced) {
            var last=withFrameNanos {it}
            while(clock.elapsed<clock.duration(duration).coerceIn(1,TextMotionCap)) {
                val now=withFrameNanos {it}
                if(clock.preparing.isEmpty())clock.elapsed=(clock.elapsed+(now-last).coerceAtLeast(0)/1_000_000f).coerceAtMost(TextMotionCap.toFloat())
                last=now
            }
            clock.elapsed=TextMotionCap.toFloat()
            if(replaySeconds in 10..30) {
                kotlinx.coroutines.delay(replaySeconds*1000L)
                clock.replay()
            }
        }
    }
    CompositionLocalProvider(LocalTextMotion provides TextMotionContext(message,clock),content=content)
}

internal fun motionOffsets(value:RichText,revealed:Set<Int>,offset:Int):Int {
    var delta=0
    for(span in value.spans) if(span.reveal.isNotEmpty() && span.start !in revealed && span.end<=offset) {
        delta+=revealPlaceholder(span.reveal).length-(span.end-span.start)
    }
    return offset+delta
}

internal val InkBrush=15.dp
internal val InkCell=8.dp

/** Per-line rectangles a reveal span occupies in the laid-out presentation. */
internal fun revealRects(value:RichText,revealed:Set<Int>,layout:TextLayoutResult,span:RichSpan,inflate:Float):List<Rect> {
    val start=motionOffsets(value,revealed,span.start)
    val end=if(span.start in revealed)motionOffsets(value,revealed,span.end) else start+revealPlaceholder(span.reveal).length
    if(start<0 || start>=end || end>layout.layoutInput.text.length)return emptyList()
    return (layout.getLineForOffset(start)..layout.getLineForOffset(end-1)).mapNotNull {line->
        val from=maxOf(start,layout.getLineStart(line))
        val to=minOf(end,layout.getLineEnd(line,true))
        if(from>=to)null else {
            val a=layout.getHorizontalPosition(from,true);val b=layout.getHorizontalPosition(to,true)
            Rect(minOf(a,b)-inflate,layout.getLineTop(line),maxOf(a,b)+inflate,layout.getLineBottom(line))
        }
    }.filter {it.width>0f && it.height>0f}
}
private fun key(x:Int,y:Int)=(x.toLong() shl 32) or (y.toLong() and 0xffffffffL)
internal fun inkCells(rects:List<Rect>,cell:Float):Set<Long> = buildSet {
    if(cell<=0f)return@buildSet
    rects.forEach {rect->
        var x=floor(rect.left/cell).toInt()
        while(x*cell<rect.right) {
            var y=floor(rect.top/cell).toInt()
            while(y*cell<rect.bottom) {add(key(x,y));y++}
            x++
        }
    }
}
internal fun brushedCells(point:Offset,radius:Float,cell:Float,eligible:Set<Long>):Set<Long> = buildSet {
    if(cell<=0f || eligible.isEmpty())return@buildSet
    var x=floor((point.x-radius)/cell).toInt()
    while(x*cell<=point.x+radius) {
        var y=floor((point.y-radius)/cell).toInt()
        while(y*cell<=point.y+radius) {
            val dx=(x+.5f)*cell-point.x;val dy=(y+.5f)*cell-point.y
            if(dx*dx+dy*dy<radius*radius)key(x,y).takeIf {it in eligible}?.let(::add)
            y++
        }
        x++
    }
}

private class InkGrain(val x:Float,val y:Float,val radius:Float,val alpha:Float,val phase:Float,val speed:Float)
private class InkVeil(val span:RichSpan,val rects:List<Rect>,val grain:List<InkGrain>)

/** Scratch conceals under drifting grain brushed away by the finger; spoiler under one flat slab wiped off by a tap. */
@Composable
internal fun revealVeil(value:RichText,revealed:Set<Int>,layout:TextLayoutResult?,ink:Color,surface:Color,brushed:List<Offset>,wiping:Int?,wipe:Float):Modifier {
    val hidden=value.spans.filter {it.reveal.isNotEmpty() && (it.start !in revealed || it.start==wiping)}
    if(hidden.isEmpty() || layout==null)return Modifier
    val motion=LocalMotion.current
    val still=motion.reduced || !LocalAppearance.current.messageEffects || !LocalMotionVisible.current
    val drift=if(still)null else rememberInfiniteTransition("Invisible ink").animateFloat(0f,2f*PI.toFloat(),motion.loop(MotionLoop*6),label="Ink drift")
    return Modifier.drawWithCache {
        val unit=1.dp.toPx()
        var budget=360
        val veils=hidden.map {span->
            val scratch=span.reveal=="scratch"
            val rects=revealRects(value,revealed,layout,span,if(scratch)2f*unit else 0f)
            InkVeil(span,rects,if(!scratch)emptyList() else buildList {
                var state=((span.start.toLong()*2654435761L) xor value.text.hashCode().toLong()) and 0xffffffffL
                fun next():Float {state=(state*1664525L+1013904223L) and 0xffffffffL;return state/4294967296f}
                rects.forEach {rect->
                    val count=minOf(budget,(rect.width*rect.height/(unit*unit*34f)).toInt().coerceIn(0,220))
                    budget-=count
                    repeat(count) {add(InkGrain(rect.left+next()*rect.width,rect.top+next()*rect.height,(.30f+next()*.75f)*unit,.24f+next()*.57f,next()*2f*PI.toFloat(),.55f+next()*.45f))}
                }
            })
        }
        val slab=lerp(surface,ink,.22f)
        val corner=CornerRadius(6.dp.toPx())
        val brush=InkBrush.toPx()
        onDrawWithContent {
            drawContent()
            val time=drift?.value ?: 0f
            veils.forEach {veil->
                val gone=if(veil.span.start==wiping)wipe else 0f
                if(gone>=1f)return@forEach
                if(veil.span.reveal=="scratch") {
                    fun grains() {veil.grain.forEach {grain->
                        val alpha=grain.alpha*(.81f+.19f*sin(time*1.8f+grain.phase))*(1f-gone)
                        drawCircle(ink.copy(alpha=alpha.coerceIn(0f,1f)),grain.radius,
                            Offset(grain.x+sin(time*.8f*grain.speed+grain.phase)*.72f*unit,grain.y+cos(time*.7f*grain.speed+grain.phase)*.66f*unit))
                    }}
                    if(brushed.isEmpty())grains()
                    else clipPath(Path().apply {brushed.forEach {addOval(Rect(it.x-brush,it.y-brush,it.x+brush,it.y+brush))}},ClipOp.Difference) {grains()}
                } else veil.rects.forEach {rect->
                    val left=rect.left+rect.width*gone
                    if(left<rect.right)drawRoundRect(slab,Offset(left,rect.top+unit),Size(rect.right-left,(rect.height-2f*unit).coerceAtLeast(1f)),corner)
                }
            }
        }
    }
}

private data class MotionCell(val path:Path,val clip:Path,val center:Offset,val height:Float,val line:Int,val scale:Float,val ink:Color,val run:TextMotion,val index:Int,val count:Int,val seed:Long,val easing:Easing)

private fun springRest(seconds:Float,run:TextMotion):Float {
    val half=run.damping.coerceIn(0,1000)/2f
    val difference=run.stiffness.coerceIn(1,10000)-half*half
    return when {
        difference>0f->{val w=sqrt(difference);exp(-half*seconds)*(cos(w*seconds)+half/w*sin(w*seconds))}
        difference<0f->{val w=sqrt(-difference);val a=-half+w;val b=-half-w;(b*exp(a*seconds)-a*exp(b*seconds))/(b-a)}
        else->exp(-half*seconds)*(1f+half*seconds)
    }.coerceIn(-1f,1f)
}

@Composable
internal fun textMotion(value:RichText,revealed:Set<Int>,layout:TextLayoutResult?,color:Color,surface:Color=Color.Unspecified):Modifier {
    val context=LocalTextMotion.current
    if(value.motion.isEmpty())return Modifier
    val source=LocalTextMotionSeeds.current
    val random=value.motion.any {it.kind in listOf("scatter","sparkle","glitch","assemble")}
    val seeds=remember(context?.message,source,random) {if(random && source!=null && context!=null)source("${context.message}/192").split(',').mapNotNull(String::toLongOrNull) else emptyList()}
    val playing=context!=null && !LocalMotion.current.reduced && LocalAppearance.current.messageEffects && (!random || seeds.size==192)
    val flipped=value.motion.any {it.kind=="flip"}
    if(!playing && !flipped)return Modifier
    val layer=rememberGraphicsLayer()
    val glowLayer=if(LocalMotionBlur.current && value.motion.any {it.kind=="glow"})rememberGraphicsLayer() else null
    return Modifier.drawWithCache {
        var seedIndex=0
        val palette=HashMap<String,Color>()
        val ground=surface.takeOrElse {color}
        val cells=value.motion.flatMap {run->
            run.units.mapIndexedNotNull {index,unit->
                val seed=seeds.getOrElse((seedIndex++).coerceAtMost(191)) {0L}
                val start=motionOffsets(value,revealed,unit.first)
                val end=motionOffsets(value,revealed,unit.second)
                if(layout==null || start<0 || end>layout.layoutInput.text.length || start>=end ||
                    value.spans.any {it.reveal.isNotEmpty() && it.start !in revealed && it.start<unit.second && it.end>unit.first})null
                else {
                    val path=layout.getPathForRange(start,end)
                    val bounds=path.getBounds()
                    val line=layout.getLineForOffset(start)
                    val easing=if(run.easing.size==4)CubicBezierEasing(run.easing[0],run.easing[1],run.easing[2],run.easing[3]) else LinearEasing
                    val span=value.spans.lastOrNull {it.start<=unit.first && it.end>unit.first}
                    // Selection rectangles clip an italic's overhang the moment a glyph moves, so widen the drawn slice only.
                    val overhang=if(span!=null && "italic" in span.flags)bounds.height*.12f else 0f
                    val ink=span?.colors?.takeIf {it.isNotEmpty()}?.let {names->gradientStop(names.map {name->palette.getOrPut(name) {textColor(name,ground)}},index,run.units.size)} ?: color
                    MotionCell(path,if(overhang<=0f)path else Path().apply {addRect(Rect(bounds.left-overhang,bounds.top,bounds.right+overhang,bounds.bottom))},
                        bounds.center,(layout.getLineBottom(line)-layout.getLineTop(line)).coerceAtLeast(1f),line,1f+(span?.size ?: 0)*.12f,ink,run,index,run.units.size,seed,easing)
                }
            }
        }.take(192)
        // Amplitude is thousandths of an em; normalise the line box by the largest span sharing the line.
        val lineScale=cells.groupBy {it.line}.mapValues {(_,group)->group.maxOf {it.scale}}
        val mask=Path().apply {cells.forEach {addPath(it.path)}}
        val glow=cells.filter {it.run.kind=="glow"}
        val glowMask=Path().apply {glow.forEach {addPath(it.path)}}
        val glowPaint=Paint()
        onDrawWithContent {
            val elapsed=if(playing)checkNotNull(context).clock.elapsed else TextMotionCap.toFloat()
            if(cells.isEmpty() || (!flipped && (elapsed>=TextMotionCap.toFloat() || cells.all {elapsed>=it.run.duration})) || size.width>4096 || size.height>4096 || size.width*size.height>4_000_000f)drawContent()
            else {
                layer.record {this@onDrawWithContent.drawContent()}
                clipPath(mask,ClipOp.Difference) {drawLayer(layer)}
                glow.firstOrNull()?.let {cell->
                    val p=(elapsed/cell.run.duration.coerceAtLeast(1)).coerceIn(0f,1f)
                    val envelope=sin(p*PI.toFloat()).coerceAtLeast(0f)
                    if(envelope>0f) {
                        val radius=cell.run.displacement/1000f*cell.height*envelope
                        if(glowLayer!=null) {
                            glowLayer.record {clipPath(glowMask) {this@onDrawWithContent.drawContent()}}
                            glowLayer.renderEffect=BlurEffect(radius.coerceAtLeast(.1f),radius.coerceAtLeast(.1f),TileMode.Decal)
                            glowLayer.alpha=.65f*envelope
                            drawLayer(glowLayer)
                        } else repeat(3) {ring->
                            val r=radius*(ring+1)/3f
                            glowPaint.alpha=(.03f/(ring+1))*envelope
                            repeat(8) {i->
                                val angle=i*PI.toFloat()/4f
                                drawContext.canvas.saveLayer(Rect(-r,-r,size.width+r,size.height+r),glowPaint)
                                withTransform({translate(cos(angle)*r,sin(angle)*r)}) {clipPath(glowMask) {drawLayer(layer)}}
                                drawContext.canvas.restore()
                            }
                        }
                    }
                }
                cells.forEach {cell->
                    val run=cell.run
                    val raw=(elapsed/run.duration.coerceAtLeast(1)).coerceIn(0f,1f)
                    val p=cell.easing.transform(raw)
                    val phase=p*2f*PI.toFloat()*run.cycles
                    val envelope=sin(p*PI.toFloat()).coerceAtLeast(0f)
                    val distance=run.displacement/1000f*cell.height*cell.scale/(lineScale[cell.line] ?: 1f)
                    val random=((cell.seed and 65535)/32767.5f)-1f
                    val other=(((cell.seed ushr 16) and 65535)/32767.5f)-1f
                    var dx=0f;var dy=0f;var angle=if(run.kind=="flip")p*run.rotation else 0f;var sx=1f;var sy=1f
                    var show=true
                    if(p<1f)when(run.kind) {
                        "shake"->dx=sin(phase)*distance*envelope
                        "wave"->dy=sin(phase-cell.index*run.stagger/1000f*2f*PI.toFloat())*distance*envelope
                        "pulse"->{
                            val beat=(raw*run.cycles)%1f
                            val response=if(beat<.2f)-.35f*sin(beat/.2f*PI.toFloat()) else sin(((beat-.2f)/.12f).coerceAtMost(1f)*PI.toFloat()/2f)*springRest((beat-.2f)*run.duration/1000f/run.cycles.coerceAtLeast(1),run)
                            sx=1f+(run.scale-1000)/1000f*response*envelope;sy=sx
                        }
                        "typewriter"->{val step=minOf(run.stagger.takeIf {it>0}?.toFloat() ?: Float.MAX_VALUE,run.duration.toFloat()/cell.count.coerceAtLeast(1));show=elapsed>=(cell.index+1)*step}
                        "scatter"->{val rest=if(p<.12f)sin(p/.12f*PI.toFloat()/2f) else springRest((p-.12f)*run.duration/1000f,run);dx=random*distance*rest;dy=other*distance*rest;angle=random*run.rotation*rest}
                        "assemble"->{val lead=minOf(cell.index.toFloat()*run.stagger,run.duration*.35f);val rest=if(elapsed<=lead)1f else springRest((elapsed-lead)/1000f,run);dx=random*distance*rest;dy=other*distance*rest;angle=random*run.rotation*rest}
                        "barrel"->{dy=-distance*envelope;sy=cos(p*run.rotation*PI.toFloat()/180f);sx=.9f+.1f*abs(sy)}
                        "glitch"->{dx=if(sin(phase)>0)distance*random*envelope else -distance*random*envelope;dy=other*distance*envelope}
                        "sparkle"->repeat(run.particles.coerceIn(0,32)) {particle->if(particle%cell.count.coerceAtLeast(1)==cell.index) {
                            val lifetime=run.particleLifetime.coerceAtLeast(1)
                            val start=particle*(run.duration-lifetime).coerceAtLeast(0).toFloat()/(run.particles-1).coerceAtLeast(1)
                            val life=((elapsed-start)/lifetime).coerceIn(0f,1f)
                            val strength=sin(life*PI.toFloat()).coerceAtLeast(0f)
                            val theta=particle*2.399963f+random*PI.toFloat()
                            val center=cell.center+Offset(cos(theta),sin(theta))*distance*(.4f+life*.6f)
                            val radius=cell.height*.08f*strength
                            drawLine(cell.ink.copy(alpha=strength),center-Offset(radius,0f),center+Offset(radius,0f),1.5.dp.toPx())
                            drawLine(cell.ink.copy(alpha=strength),center-Offset(0f,radius),center+Offset(0f,radius),1.5.dp.toPx())
                        }
                        }
                    }
                    if(show) withTransform({translate(dx,dy);rotate(angle,cell.center);scale(sx,sy,cell.center)}) {
                        clipPath(cell.clip) {drawLayer(layer)}
                    }
                }
            }
        }
    }
}
