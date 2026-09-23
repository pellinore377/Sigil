package org.sigil

import androidx.compose.runtime.*
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.platform.LocalDensity
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
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.TextUnitType
import androidx.compose.ui.unit.sp
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
    // Automatic replay follows a message that played in this session, never history settled on open.
    var autoplay=fresh
    fun replay() {elapsed=0f;materialDuration=0;generation++;autoplay=true}
}
private const val ArrivalBurst=8
internal class MotionLedger {
    private var initialized=false
    private var previous=emptySet<String>()
    private val entries=linkedMapOf<String,TextPlayback>()
    fun update(ids:List<String>,loaded:Boolean,allowNew:Boolean,animated:Set<String> = ids.toSet()) {
        if(!loaded)return
        // Nothing known yet and a handful arriving is a new conversation; a whole page at once is history.
        val boundary=if(!initialized || !allowNew)0 else ids.indexOfFirst {it in previous}.let {if(it>=0)it else if(previous.isEmpty() && ids.size<=ArrivalBurst)ids.size else 0}
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
    maxOf(if(p.chart!=null)ChartMotionMillis else 0,if(p.diagram!=null)DiagramMotionMillis else 0,if(p.utility?.kind=="progress")ProgressMotionMillis else 0,if(p.table!=null)TableMotionMillis else 0,if(p.utility?.kind=="rating")RatingMotionMillis else 0,p.utility?.motion?.let(::randomizerDuration) ?: 0,texts.maxOfOrNull {t->t.motion.maxOfOrNull {if(it.kind=="typewriter" && it.stagger>0)minOf(it.duration,it.stagger*it.units.size) else it.duration} ?: 0} ?: 0)
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
            if(clock.autoplay && replaySeconds in 10..30) {
                kotlinx.coroutines.delay(replaySeconds*1000L)
                clock.replay()
            }
        }
    }
    CompositionLocalProvider(LocalTextMotion provides TextMotionContext(message,clock),content=content)
}

internal fun motionOffsets(value:RichText,revealed:Set<Int>,offset:Int):Int {
    var delta=0
    for(span in value.spans) {
        // A counted redaction is drawn as a run of its own length over a single placeholder in the body.
        if(span.redaction>0 && span.end<=offset) delta+=span.redaction-(span.end-span.start)
    }
    return offset+delta
}

internal val InkBrush=18.dp
internal val InkCell=8.dp

/** Per-line rectangles a reveal span occupies in the laid-out presentation. */
/** Horizontal extent of [start, end) on one line. An offset sitting on a line break belongs to the next line,
 *  so a span that runs to the break takes the line's own edge instead of a position from the line after it. */
internal fun lineSpanEdges(layout:TextLayoutResult,line:Int,start:Int,end:Int):Pair<Float,Float>? {
    val lineStart=layout.getLineStart(line);val lineEnd=layout.getLineEnd(line,true)
    val from=maxOf(start,lineStart);val to=minOf(end,lineEnd)
    if(from>=to)return null
    val a=if(from<=lineStart)layout.getLineLeft(line) else layout.getHorizontalPosition(from,true)
    val b=if(to>=lineEnd)layout.getLineRight(line) else layout.getHorizontalPosition(to,true)
    return minOf(a,b) to maxOf(a,b)
}
internal fun revealRects(value:RichText,revealed:Set<Int>,layout:TextLayoutResult,span:RichSpan,inflate:Float):List<Rect> {
    val start=motionOffsets(value,revealed,span.start)
    val end=motionOffsets(value,revealed,span.end)
    if(start<0 || start>=end || end>layout.layoutInput.text.length)return emptyList()
    return (layout.getLineForOffset(start)..layout.getLineForOffset(end-1)).mapNotNull {line->
        lineSpanEdges(layout,line,start,end)?.let {(a,b)->Rect(a-inflate,layout.getLineTop(line),b+inflate,layout.getLineBottom(line))}
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

private class Tile(val rect:Rect,val phase:Float,val speed:Float)
private class Veil(val span:RichSpan,val rects:List<Rect>,val cover:List<Rect>,val clip:Path,val tiles:List<Tile>)

/** Spoiler is the chrome's glass laid over the text, clearing from the touch. Scratch is a mosaic of shimmering tiles the finger knocks out. */
@Composable
internal fun revealVeil(value:RichText,revealed:Set<Int>,layout:TextLayoutResult?,ink:Color,surface:Color,brushed:List<Offset>,wiping:Int?,wipe:Float,tap:Offset):Modifier {
    val hidden=value.spans.filter {it.reveal.isNotEmpty() && (it.start !in revealed || it.start==wiping)}
    if(hidden.isEmpty() || layout==null)return Modifier
    val motion=LocalMotion.current
    val still=motion.reduced || !LocalAppearance.current.messageEffects || !LocalMotionVisible.current
    val drift=if(still)null else rememberInfiniteTransition("Invisible ink").animateFloat(0f,2f*PI.toFloat(),motion.loop(MotionLoop*6),label="Ink drift")
    // The same glass as the floating header and footer: a 14dp blur under surfaceContainerHigh at 82%.
    val glassTint=MaterialTheme.colorScheme.surfaceContainerHigh
    val glassRadius=with(LocalDensity.current) {14.dp.toPx()}
    val glassEffect=remember(glassRadius) {BlurEffect(glassRadius,glassRadius,TileMode.Clamp)}
    val frost=if(LocalMotionBlur.current && glassEffect.isSupported() && hidden.any {it.reveal=="spoiler"})rememberGraphicsLayer() else null
    return Modifier.drawWithCache {
        val unit=1.dp.toPx()
        val em=(layout.layoutInput.style.fontSize.takeIf {it.type==TextUnitType.Sp}?.toPx()) ?: 16.sp.toPx()
        val cell=6.dp.toPx()
        var budget=1400
        val veils=hidden.map {span->
            val start=motionOffsets(value,revealed,span.start);val end=motionOffsets(value,revealed,span.end)
            // The same box as a redaction: the text's own extent, level with the em.
            val lines=if(start<0 || start>=end || end>layout.layoutInput.text.length)emptyList() else (layout.getLineForOffset(start)..layout.getLineForOffset(end-1)).toList()
            val rects=lines.mapNotNull {line->lineSpanEdges(layout,line,start,end)?.let {(a,b)->val base=layout.getLineBaseline(line);Rect(a,base-em*.89f,b,base+em*.29f)}}.filter {it.width>0f}
            // Ascenders and the corners the squircle rounds off would show: the run's whole line box is painted in the
            // bubble's own colour beneath the veil, invisible against it, so the bar can keep the redaction's box.
            val cover=lines.mapNotNull {line->lineSpanEdges(layout,line,start,end)?.let {(a,b)->Rect(a-unit,layout.getLineTop(line),b+unit,layout.getLineBottom(line))}}
            val clip=Path().apply {rects.forEach {addPath(squirclePath(it,em*.30f))}}
            Veil(span,rects,cover,clip,if(span.reveal!="scratch")emptyList() else buildList {
                var state=((span.start.toLong()*2654435761L) xor value.text.hashCode().toLong()) and 0xffffffffL
                fun next():Float {state=(state*1664525L+1013904223L) and 0xffffffffL;return state/4294967296f}
                rects.forEach {rect->
                    val cols=ceil(rect.width/cell).toInt();val rows=ceil(rect.height/cell).toInt()
                    for(y in 0 until rows)for(x in 0 until cols) {
                        if(budget==0)return@forEach
                        budget--
                        val left=rect.left+x*cell;val top=rect.top+y*cell
                        // Whole-number speeds keep every tile continuous when the drift loops.
                        add(Tile(Rect(left,top,minOf(rect.right,left+cell),minOf(rect.bottom,top+cell)),next()*2f*PI.toFloat(),if(next()<.5f)1f else 2f))
                    }
                }
            })
        }
        val low=lerp(surface,ink,.12f);val high=lerp(surface,ink,.28f)
        val brush=InkBrush.toPx()
        onDrawWithContent {
            drawContent()
            val time=drift?.value ?: 0f
            veils.forEach {veil->
                val gone=if(veil.span.start==wiping)wipe else 0f
                if(gone>=1f || veil.rects.isEmpty())return@forEach
                val bounds=veil.rects.reduce {a,c->a.expandToInclude(c)}
                if(veil.span.reveal=="scratch") {
                    // Soft-edged holes punched out of the mosaic, as on the study page; the layer holds no blur, so it is safe here.
                    drawContext.canvas.saveLayer(bounds.inflate(brush+unit*4f),Paint())
                    veil.cover.forEach {drawRect(surface.copy(alpha=1f-gone),it.topLeft,it.size)}
                    clipPath(veil.clip) {veil.tiles.forEach {tile->
                        val k=(sin(time*tile.speed+tile.phase)+1f)/2f
                        drawRect(lerp(low,high,k).copy(alpha=1f-gone),tile.rect.topLeft,tile.rect.size)
                    }}
                    brushed.forEach {p->drawCircle(Brush.radialGradient(0f to Color.Black,.5f to Color.Black,1f to Color.Transparent,center=p,radius=brush),brush,p,blendMode=BlendMode.DstOut)}
                    drawContext.canvas.restore()
                } else {
                    // Defrosting clears a circle that grows from the touch while the glass fades.
                    val reach=maxOf(hypot(tap.x-bounds.left,tap.y-bounds.top),hypot(bounds.right-tap.x,tap.y-bounds.top),hypot(tap.x-bounds.left,bounds.bottom-tap.y),hypot(bounds.right-tap.x,bounds.bottom-tap.y))
                    val clear=Path().apply {if(gone>0f) {val r=reach*gone;addOval(Rect(tap.x-r,tap.y-r,tap.x+r,tap.y+r))}}
                    clipPath(clear,ClipOp.Difference) {veil.cover.forEach {drawRect(surface.copy(alpha=1f-gone),it.topLeft,it.size)}}
                    clipPath(veil.clip) {clipPath(clear,ClipOp.Difference) {
                        if(frost!=null) {
                            // Ground and text together, so the blurred capture is opaque, as the chrome's backdrop is.
                            frost.record {drawRect(surface);clipPath(veil.clip) {this@onDrawWithContent.drawContent()}}
                            frost.renderEffect=glassEffect
                            frost.alpha=1f-gone
                            drawLayer(frost)
                        }
                        drawRect(glassTint.copy(alpha=.62f*(1f-gone)),topLeft=Offset(bounds.left,bounds.top),size=Size(bounds.width,bounds.height))
                    }}
                }
            }
        }
    }
}

/** One laid-out grapheme of an animated run. */
private class MotionCell(val path:Path,val clip:Path,val bounds:Rect,val center:Offset,val height:Float,val fontPx:Float,val ink:Color,val index:Int,val baseline:Float)
private class SparkleParticle(val x:Float,val y:Float,val size:Float,val dx:Float,val dy:Float,val delay:Float,val ink:Color)

private const val MotionReferenceEm=64f
/** (ascent + descent) / 2 from each bundled font's hhea table, in em above the baseline. */
private fun fontMidline(font:String)=if(font=="Newsreader").235f else .340f
private val MotionSmooth=CubicBezierEasing(.4f,0f,.2f,1f)

/** The reference player's LCG; renderers must not seed from wall time. */
private class MotionRandom(seed:Long) {
    private var state=seed and 0xffffffffL
    fun next():Float {state=(state*1664525L+1013904223L) and 0xffffffffL;return state/4294967296f}
}
private fun ampScale(fontPx:Float)=(fontPx/MotionReferenceEm).coerceIn(.28f,1.8f)
private fun smoothstep(t:Float)=t*t*(3f-2f*t)
/** Unit-step response of the reference underdamped spring. */
private fun springStep(u:Float)=exp(-7.8f*u)*(cos(9.4f*u)+7.8f/9.4f*sin(9.4f*u))
private fun lerpFrames(t:Float,offsets:FloatArray,values:FloatArray):Float {
    if(t<=offsets.first())return values.first()
    if(t>=offsets.last())return values.last()
    var i=1
    while(i<offsets.size && offsets[i]<t)i++
    val a=offsets[i-1];val b=offsets[i]
    return values[i-1]+(values[i]-values[i-1])*if(b>a)(t-a)/(b-a) else 0f
}

/** Per-run timeline, random draw and derived geometry, all fixed at layout time. */
private class MotionPlan(val run:TextMotion,val cells:List<MotionCell>,seed:Long) {
    val n=cells.size
    val tail=if(n<2)0f else minOf(640f,(n-1)*run.stagger.toFloat())
    val span=(run.duration-tail).coerceAtLeast(1f)
    val bounds=cells.fold(cells.first().bounds) {a,c->a.expandToInclude(c.bounds)}
    fun delay(i:Int)=if(n<2)0f else tail*i/(n-1)
    fun local(elapsed:Float,i:Int)=((elapsed-delay(i))/span).coerceIn(0f,1f)

    val offsetX=FloatArray(n);val offsetY=FloatArray(n);val spin=FloatArray(n)
    val faulted=BooleanArray(n);val faultX=FloatArray(n);val faultY=FloatArray(n);val symbol=arrayOfNulls<String>(n)
    val particles=ArrayList<SparkleParticle>()
    val reveal=FloatArray(n)

    init {
        val rand=MotionRandom(seed)
        when(run.kind) {
            "scatter","assemble"->for(i in 0 until n) {
                val amp=run.amplitude*ampScale(cells[i].fontPx)
                if(run.kind=="assemble") {
                    // sort: a scattered band resolves left to right.
                    offsetX[i]=(rand.next()-.5f)*amp*1.7f
                    offsetY[i]=(if(i%2==1)1f else -1f)*amp*(.35f+rand.next()*.4f)
                    spin[i]=(rand.next()-.5f)*66f*.4f
                } else {
                    val phase=rand.next()*2f*PI.toFloat()
                    offsetX[i]=cos(phase)*amp*(.6f+rand.next()*.5f)
                    offsetY[i]=sin(phase)*amp*(.5f+rand.next()*.5f)
                    spin[i]=(rand.next()-.5f)*46f
                }
            }
            "glitch"->{
                val glyphs=run.substitutions.map(Char::toString)
                for(i in 0 until n) {
                    val amp=run.amplitude*ampScale(cells[i].fontPx)
                    symbol[i]=glyphs.getOrNull((rand.next()*glyphs.size).toInt().coerceIn(0,glyphs.size-1))
                    faulted[i]=i%3!=0
                    faultX[i]=(if(rand.next()<.5f)-1f else 1f)*amp
                    faultY[i]=(if(rand.next()<.5f)-1f else 1f)*amp
                }
            }
            "sparkle"->repeat(run.particles.toInt().coerceIn(0,64)) {i->
                val cell=cells[(rand.next()*n).toInt().coerceIn(0,n-1)]
                val scale=ampScale(cell.fontPx)
                val amp=run.amplitude*scale
                val top=i%2==0
                val x=cell.bounds.left+cell.bounds.width*(.15f+rand.next()*.7f)
                val y=cell.bounds.top+cell.bounds.height*(if(top).13f else .83f)
                val size=(12f+rand.next()*6f)*scale*1.7f
                val angle=rand.next()*2f*PI.toFloat()
                particles+=SparkleParticle(x,y,size,cos(angle)*amp,(if(top)-1f else 1f)*amp*(.4f+rand.next()*.7f),
                    i.toFloat()/run.particles.coerceAtLeast(1)*run.duration*.62f,cell.ink)
            }
            "typewriter"->{
                // Equal weights: a glyph appears at its own position along the run.
                val from=cells.first().bounds.left;val to=cells.last().bounds.right
                for(i in 0 until n) reveal[i]=.075f+(if(to>from)(cells[i].bounds.right-from)/(to-from) else (i+1f)/n)*.84f
            }
        }
    }
}
private fun Rect.expandToInclude(other:Rect)=Rect(minOf(left,other.left),minOf(top,other.top),maxOf(right,other.right),maxOf(bottom,other.bottom))

/** Four-point star, matching the reference particle clip path. */
private fun starPath(center:Offset,size:Float):Path {
    val points=floatArrayOf(.50f,0f, .61f,.37f, 1f,.50f, .61f,.61f, .50f,1f, .39f,.61f, 0f,.50f, .39f,.37f)
    return Path().apply {
        for(i in 0 until 8) {
            val x=center.x+(points[i*2]-.5f)*size;val y=center.y+(points[i*2+1]-.5f)*size
            if(i==0)moveTo(x,y) else lineTo(x,y)
        }
        close()
    }
}

@Composable
internal fun textMotion(value:RichText,revealed:Set<Int>,layout:TextLayoutResult?,color:Color,surface:Color=Color.Unspecified):Modifier {
    val context=LocalTextMotion.current
    if(value.motion.isEmpty())return Modifier
    val source=LocalTextMotionSeeds.current
    val seeded=value.motion.any {it.kind in listOf("scatter","assemble","sparkle","glitch")}
    val seeds=remember(context?.message,source,seeded) {if(seeded && source!=null && context!=null)source("${context.message}/192").split(',').mapNotNull(String::toLongOrNull) else emptyList()}
    // Flip is static, not motion: it renders with no playback context and under reduced motion alike.
    val static=value.motion.any {it.kind=="flip"}
    val playing=context!=null && !LocalMotion.current.reduced && LocalAppearance.current.messageEffects && !(seeded && seeds.size!=192)
    if(!playing && !static)return Modifier
    val layer=rememberGraphicsLayer()
    val bloom=if(LocalMotionBlur.current && value.motion.any {it.kind=="glow"}) List(3) {rememberGraphicsLayer()} else null
    val measurer=rememberTextMeasurer()
    val codeFont=LocalCodeFont.current
    val flipPivot=fontMidline(LocalAppearance.current.font)
    return Modifier.drawWithCache {
        val palette=HashMap<String,Color>()
        val ground=surface.takeOrElse {color}
        val fallbackEm=(layout?.layoutInput?.style?.fontSize?.takeIf {it.type==TextUnitType.Sp}?.toPx()) ?: 16.sp.toPx()
        var seedIndex=0
        val plans=value.motion.mapNotNull {run->
            val runSeed=seeds.getOrElse((seedIndex++).coerceAtMost(191)) {0L}
            val cells=run.units.mapIndexedNotNull {index,unit->
                val start=motionOffsets(value,revealed,unit.first)
                val end=motionOffsets(value,revealed,unit.second)
                if(layout==null || start<0 || end>layout.layoutInput.text.length || start>=end ||
                    value.spans.any {it.reveal.isNotEmpty() && it.start !in revealed && it.start<unit.second && it.end>unit.first})null
                else {
                    val path=layout.getPathForRange(start,end)
                    val bounds=path.getBounds()
                    val line=layout.getLineForOffset(start)
                    val span=value.spans.lastOrNull {it.start<=unit.first && it.end>unit.first}
                    // Selection rectangles clip an italic's overhang the moment a glyph moves, so widen the drawn slice only.
                    val overhang=if(span!=null && "italic" in span.flags)bounds.height*.12f else 0f
                    val ink=span?.colors?.takeIf {it.isNotEmpty()}?.let {names->gradientStop(names.map {name->palette.getOrPut(name) {textColor(name,ground)}},index,run.units.size)} ?: color
                    val box=Rect(bounds.left,layout.getLineTop(line),bounds.right,layout.getLineBottom(line))
                    MotionCell(path,if(overhang<=0f)path else Path().apply {addRect(Rect(bounds.left-overhang,bounds.top,bounds.right+overhang,bounds.bottom))},
                        box,bounds.center,box.height.coerceAtLeast(1f),fallbackEm*(1f+(span?.size ?: 0)*.12f),ink,index,layout.getLineBaseline(line))
                }
            }.take(192)
            if(cells.isEmpty())null else MotionPlan(run,cells,runSeed)
        }
        // Selection rectangles do not tile at whole pixels; a pixel of overlap keeps upright slivers from leaking through.
        val mask=Path().apply {plans.forEach {plan->plan.cells.forEach {val b=it.path.getBounds();addRect(Rect(b.left-1f,b.top,b.right+1f,b.bottom))}}}
        val still=plans.any {it.run.kind=="flip"}
        val finish=plans.maxOfOrNull {it.run.duration.toFloat()} ?: 0f
        onDrawWithContent {
            val elapsed=if(playing)checkNotNull(context).clock.elapsed else TextMotionCap.toFloat()
            if(plans.isEmpty() || (!still && elapsed>=finish) || size.width>4096 || size.height>4096 || size.width*size.height>4_000_000f) {drawContent();return@onDrawWithContent}
            layer.record {this@onDrawWithContent.drawContent()}
            clipPath(mask,ClipOp.Difference) {drawLayer(layer)}
            plans.forEach {plan->drawPlan(plan,elapsed,layer,bloom,measurer,codeFont,flipPivot)}
        }
    }
}

private fun DrawScope.drawPlan(plan:MotionPlan,elapsed:Float,layer:androidx.compose.ui.graphics.layer.GraphicsLayer,
    bloom:List<androidx.compose.ui.graphics.layer.GraphicsLayer>?,measurer:androidx.compose.ui.text.TextMeasurer,codeFont:FontFamily?,flipPivot:Float) {
    val run=plan.run
    val whole=(elapsed/plan.span).coerceIn(0f,1f)
    fun paint(cell:MotionCell)=clipPath(cell.clip) {drawLayer(layer)}

    when(run.kind) {
        // echo: a short tremor, a breath, then one smaller aftershock, on the whole line.
        "shake"->{
            val amp=run.amplitude*ampScale(plan.cells.first().fontPx)
            val steps=34
            fun sample(j:Int):Float {
                if(j<=0 || j>=steps)return 0f
                val t=j.toFloat()/steps
                val envelope=when {t<.40f->sin(t/.4f*PI.toFloat());t>.66f->sin((t-.66f)/.34f*PI.toFloat())*.48f;else->0f}
                return amp*envelope*(if(j%2==1)-1f else 1f)*(.6f+.4f*abs(sin(j*7.17f)))
            }
            val u=whole*steps;val j=u.toInt().coerceIn(0,steps)
            val dx=sample(j)+(sample(minOf(j+1,steps))-sample(j))*(u-j)
            withTransform({translate(dx,0f)}) {plan.cells.forEach(::paint)}
        }
        // elastic: squash and stretch, on the whole line, geometry only.
        "pulse"->{
            val a=run.amplitude
            val t=MotionSmooth.transform(whole)
            val offsets=floatArrayOf(0f,.18f,.39f,.61f,.8f,1f)
            val sx=lerpFrames(t,offsets,floatArrayOf(1f,1f+a*.42f,1f-a*.22f,1f+a*.14f,1f-a*.04f,1f))
            val sy=lerpFrames(t,offsets,floatArrayOf(1f,1f-a*.85f,1f+a,1f-a*.23f,1f+a*.06f,1f))
            withTransform({scale(sx,sy,plan.bounds.center)}) {plan.cells.forEach(::paint)}
        }
        // travel: light moves from character to character; the letterforms never do.
        "glow"->{
            val amp=run.amplitude*ampScale(plan.cells.first().fontPx)
            val strengths=plan.cells.map {cell->
                val t=MotionSmooth.transform(plan.local(elapsed,cell.index))
                lerpFrames(t,floatArrayOf(0f,.38f,.58f,1f),floatArrayOf(0f,1f,.7f,0f))
            }
            // A text shadow is a blur of the glyph's own alpha. We have no outlines, so blur the whole
            // rendered line -- that is the same thing -- and vary intensity with a gradient alpha mask
            // whose stops sit on the glyph centres. Nothing is clipped, so nothing has a straight edge.
            val lit=plan.cells.zip(strengths).filter {it.second>0f}
            if(bloom!=null && lit.isNotEmpty()) {
                val rings=listOf(.15f,.65f,1f)
                rings.forEachIndexed {ring,spread->
                    bloom[ring].record {drawLayer(layer)}
                    bloom[ring].renderEffect=BlurEffect((amp*spread).coerceAtLeast(.1f),(amp*spread).coerceAtLeast(.1f),TileMode.Decal)
                }
                val reach=amp.coerceAtLeast(.1f)
                plan.cells.zip(strengths).groupBy {it.first.bounds.top}.forEach {(_,row)->
                    if(row.none {it.second>0f})return@forEach
                    val box=row.fold(row.first().first.bounds) {a,c->a.expandToInclude(c.first.bounds)}
                    val left=box.left-reach;val right=box.right+reach
                    val top=box.top-reach;val bottom=box.bottom+reach
                    if(right<=left || bottom<=top)return@forEach
                    val stops=buildList {
                        add(0f to row.first().second)
                        row.forEach {(cell,level)->add(((cell.center.x-left)/(right-left)).coerceIn(0f,1f) to level)}
                        add(1f to row.last().second)
                    }.sortedBy {it.first}.distinctBy {it.first}
                        .map {(at,level)->at to Color.White.copy(alpha=level.coerceIn(0f,1f))}
                    val area=Rect(left,top,right,bottom)
                    drawContext.canvas.saveLayer(area,Paint())
                    rings.indices.forEach {ring->clipRect(left,top,right,bottom) {drawLayer(bloom[ring])}}
                    drawRect(Brush.horizontalGradient(*stops.toTypedArray(),startX=left,endX=right),
                        topLeft=Offset(left,top),size=Size(right-left,bottom-top),blendMode=BlendMode.DstIn)
                    drawContext.canvas.restore()
                }
            } else if(bloom==null) lit.forEach {(cell,level)->
                repeat(4) {step->
                    val angle=step*PI.toFloat()/2f
                    layer.alpha=(level*.12f).coerceIn(0f,1f)
                    withTransform({translate(cos(angle)*amp,sin(angle)*amp)}) {clipRect(cell.bounds.left,cell.bounds.top,cell.bounds.right,cell.bounds.bottom) {drawLayer(layer)}}
                }
                layer.alpha=1f
            }
            // The bloom is a shadow: it sits behind the letterforms, which never move.
            plan.cells.forEach(::paint)
        }
        // ribbon: two close ripples through a tightly staggered line.
        "wave"->plan.cells.forEach {cell->
            val amp=run.amplitude*ampScale(cell.fontPx)
            val t=plan.local(elapsed,cell.index)
            val y=if(t>=1f)0f else -sin(t*PI.toFloat()*4f)*sin(PI.toFloat()*t).pow(.75f)*amp
            withTransform({translate(0f,y)}) {paint(cell)}
        }
        // burst / sort: deterministic displacement returning to the exact original layout.
        "scatter","assemble"->plan.cells.forEach {cell->
            val i=cell.index
            val t=plan.local(elapsed,i)
            val f=when {
                t>=1f->0f
                run.kind=="assemble"->springStep(((t-.09f)/.91f).coerceIn(0f,1f))
                t<.10f->-.035f*sin(t/.10f*PI.toFloat())
                t<.34f->{val u=(t-.10f)/.24f;1f-(1f-u).pow(3)}
                t<.46f->1f+.03f*sin((t-.34f)/.12f*PI.toFloat())
                else->springStep((t-.46f)/.54f)
            }
            val alpha=if(run.kind=="assemble")minOf(1f,.20f+t*4.4f) else 1f
            withTransform({translate(plan.offsetX[i]*f,plan.offsetY[i]*f);rotate(plan.spin[i]*f,cell.center)}) {
                if(alpha>=1f)paint(cell) else {layer.alpha=alpha;paint(cell);layer.alpha=1f}
            }
        }
        // ripple: every grapheme hops and rolls on its own, left to right.
        "barrel"->plan.cells.forEach {cell->
            val scale=ampScale(cell.fontPx)
            val amp=run.amplitude*scale
            val t=plan.local(elapsed,cell.index)
            val flight=((t-.12f)/.68f).coerceIn(0f,1f)
            val arc=sin(PI.toFloat()*flight)
            val spin=360f*smoothstep(flight)
            val landing=if(t>.80f)sin((t-.8f)/.2f*2f*PI.toFloat())*exp(-(t-.8f)*18f) else 0f
            val y=-amp*arc+(if(t<.12f)sin(t/.12f*PI.toFloat())*2f*scale else -landing*amp*.13f)
            val tilt=sin(flight*2f*PI.toFloat())*22f
            withTransform({translate(0f,y);rotate(spin,cell.center);scale(cos(tilt*PI.toFloat()/180f),1f-landing*.07f,cell.center)}) {paint(cell)}
        }
        // fragment: two restrained faults substitute glyphs and displace them vertically.
        "glitch"->{
            val on=whole in .17f..<.31f || whole in .65f..<.78f
            plan.cells.forEach {cell->
                val i=cell.index
                if(!on || !plan.faulted[i]) {paint(cell);return@forEach}
                val glyph=plan.symbol[i]
                if(glyph==null) {paint(cell);return@forEach}
                val style=TextStyle(color=cell.ink,fontSize=(cell.fontPx*.9f).toSp(),fontFamily=codeFont)
                val measured=measurer.measure(glyph,style)
                drawText(measured,topLeft=Offset(cell.center.x-measured.size.width/2f+plan.faultX[i],cell.center.y-measured.size.height/2f+plan.faultY[i]))
            }
        }
        // measured: complete graphemes at an even pace, behind a blinking caret.
        "typewriter"->{
            val shown=if(whole>=.93f)plan.n else plan.reveal.count {it<=whole}
            plan.cells.forEach {cell->if(cell.index<shown)paint(cell)}
            if(whole<.96f && shown>0) {
                val last=plan.cells[(shown-1).coerceIn(0,plan.n-1)]
                val alpha=if((elapsed/420f).toInt()%2==0)1f else .28f
                drawRoundRect(last.ink.copy(alpha=alpha),Offset(last.bounds.right+2f,last.bounds.top+last.height*.14f),
                    Size(2f,last.height*.72f),CornerRadius(1f))
            }
        }
        // constellation: a handful of larger stars appear in a deliberate sequence.
        "sparkle"->{
            plan.cells.forEach(::paint)
            plan.particles.forEach {particle->
                val life=((elapsed-particle.delay)/980f).coerceIn(0f,1f)
                if(life<=0f || life>=1f)return@forEach
                val t=MotionSmooth.transform(life)
                val offsets=floatArrayOf(0f,.28f,.61f,1f)
                val alpha=lerpFrames(t,offsets,floatArrayOf(0f,.94f,.65f,0f))
                val grow=lerpFrames(t,offsets,floatArrayOf(.15f,1f,.8f,.08f))
                val turn=lerpFrames(t,offsets,floatArrayOf(0f,16f,34f,50f))
                val dx=lerpFrames(t,offsets,floatArrayOf(0f,particle.dx*.35f,particle.dx*.7f,particle.dx))
                val dy=lerpFrames(t,offsets,floatArrayOf(0f,particle.dy*.3f,particle.dy*.67f,particle.dy))
                val at=Offset(particle.x+dx,particle.y+dy)
                withTransform({rotate(turn,at)}) {drawPath(starPath(at,particle.size*grow),particle.ink.copy(alpha=alpha.coerceIn(0f,1f)))}
            }
        }
        // A rigid 180-degree turn of each line's run, as typed: advances, bearings and kerning are the
        // original's, mirrored. Pivoting on the font midline keeps the run in the same vertical band.
        "flip"->plan.cells.groupBy {it.bounds.top}.values.forEach {row->
            val box=row.fold(row.first().bounds) {acc,c->acc.expandToInclude(c.bounds)}
            val pivot=Offset(box.center.x,row.first().baseline-row.first().fontPx*flipPivot)
            withTransform({rotate(180f,pivot)}) {clipRect(box.left,box.top,box.right,box.bottom) {drawLayer(layer)}}
        }
        else->plan.cells.forEach(::paint)
    }
}
