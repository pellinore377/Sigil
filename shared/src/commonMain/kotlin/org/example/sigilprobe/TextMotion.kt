package org.sigil

import androidx.compose.runtime.*
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.*
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.compose.ui.graphics.rememberGraphicsLayer
import androidx.compose.ui.text.TextLayoutResult
import kotlin.math.*

val LocalTextMotionSeeds=staticCompositionLocalOf<((String)->String)?> {null}
val LocalMotionVisible=staticCompositionLocalOf {true}
val LocalMotionBlur=staticCompositionLocalOf {true}
internal class TextPlayback(fresh:Boolean=false) {
    var elapsed by mutableFloatStateOf(if(fresh)0f else 2000f)
    var generation by mutableIntStateOf(0)
    fun replay() {elapsed=0f;generation++}
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
    maxOf(if(p.chart!=null)ChartMotionMillis else 0,if(p.utility?.motion!=null)RandomizerMotionMillis else 0,texts.maxOfOrNull {t->t.motion.maxOfOrNull {if(it.kind=="typewriter" && it.stagger>0)minOf(it.duration,it.stagger*it.units.size) else it.duration} ?: 0} ?: 0)
} ?: 0
internal fun ChatMessage.hasMessageMotion()=messageMotionDuration()>0

@Composable
internal fun MessageMotion(message:String,clock:TextPlayback,visible:Boolean,duration:Int=2000,content:@Composable ()->Unit) {
    val active=visible && LocalMotionVisible.current
    val reduced=LocalMotion.current.reduced || !LocalAppearance.current.messageEffects
    LaunchedEffect(clock,active,reduced,clock.generation,duration) {
        if(reduced)clock.elapsed=2000f
        if(active && !reduced) {
            var last=withFrameNanos {it}
            while(clock.elapsed<duration.coerceIn(1,2000)) {
                val now=withFrameNanos {it}
                clock.elapsed=(clock.elapsed+(now-last).coerceAtLeast(0)/1_000_000f).coerceAtMost(2000f)
                last=now
            }
            clock.elapsed=2000f
        }
    }
    CompositionLocalProvider(LocalTextMotion provides TextMotionContext(message,clock),content=content)
}

internal fun motionOffsets(value:RichText,revealed:Set<Int>,offset:Int):Int {
    var delta=0
    for(span in value.spans) if(span.reveal.isNotEmpty() && span.start !in revealed && span.end<=offset) {
        delta+=(if(span.reveal=="scratch")"Scratch to reveal" else "Hidden text").length-(span.end-span.start)
    }
    return offset+delta
}

private data class MotionCell(val path:Path,val center:Offset,val height:Float,val run:TextMotion,val index:Int,val count:Int,val seed:Long,val easing:Easing)

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
internal fun textMotion(value:RichText,revealed:Set<Int>,layout:TextLayoutResult?,color:Color):Modifier {
    val context=LocalTextMotion.current
    if(value.motion.isEmpty())return Modifier
    val source=LocalTextMotionSeeds.current
    val random=value.motion.any {it.kind in listOf("scatter","sparkle","glitch")}
    val seeds=remember(context?.message,source,random) {if(random && source!=null && context!=null)source("${context.message}/192").split(',').mapNotNull(String::toLongOrNull) else emptyList()}
    val playing=context!=null && !LocalMotion.current.reduced && LocalAppearance.current.messageEffects && (!random || seeds.size==192)
    val flipped=value.motion.any {it.kind=="flip"}
    if(!playing && !flipped)return Modifier
    val layer=rememberGraphicsLayer()
    val glowLayer=if(LocalMotionBlur.current && value.motion.any {it.kind=="glow"})rememberGraphicsLayer() else null
    return Modifier.drawWithCache {
        var seedIndex=0
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
                    MotionCell(path,bounds.center,(layout.getLineBottom(line)-layout.getLineTop(line)).coerceAtLeast(1f),run,index,run.units.size,seed,easing)
                }
            }
        }.take(192)
        val mask=Path().apply {cells.forEach {addPath(it.path)}}
        val glow=cells.filter {it.run.kind=="glow"}
        val glowMask=Path().apply {glow.forEach {addPath(it.path)}}
        val glowPaint=Paint()
        onDrawWithContent {
            val elapsed=if(playing)checkNotNull(context).clock.elapsed else 2000f
            if(cells.isEmpty() || (!flipped && (elapsed>=2000f || cells.all {elapsed>=it.run.duration})) || size.width>4096 || size.height>4096 || size.width*size.height>4_000_000f)drawContent()
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
                    val distance=run.displacement/1000f*cell.height
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
                            drawLine(color.copy(alpha=strength),center-Offset(radius,0f),center+Offset(radius,0f),1.5f)
                            drawLine(color.copy(alpha=strength),center-Offset(0f,radius),center+Offset(0f,radius),1.5f)
                        }
                        }
                    }
                    if(show) withTransform({translate(dx,dy);rotate(angle,cell.center);scale(sx,sy,cell.center)}) {
                        clipPath(cell.path) {drawLayer(layer)}
                    }
                }
            }
        }
    }
}
