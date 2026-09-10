package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.*
import androidx.compose.ui.geometry.*
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.*
import androidx.compose.ui.window.*
import kotlin.math.*

private fun DiagramContent.descendants(node: Int): Set<Int> {
    val seen = mutableSetOf<Int>()
    val queue = ArrayDeque<Int>(); queue.add(node)
    while (queue.isNotEmpty()) { val at = queue.removeFirst(); if (seen.add(at)) edges.filter { it.from == at }.forEach { queue.add(it.to) } }
    return seen
}

@Composable
internal fun DiagramCard(diagram: DiagramContent) {
    var expanded by remember(diagram) { mutableStateOf(false) }
    var focused by remember(diagram) { mutableStateOf<Int?>(null) }
    var collapsed by remember(diagram) { mutableStateOf(emptySet<Int>()) }
    var branch by remember(diagram) { mutableStateOf<Int?>(null) }
    var outline by remember(diagram) { mutableStateOf(false) }
    val hidden = remember(diagram, collapsed, branch) { collapsed.flatMap { diagram.descendants(it) - it }.toSet() + (branch?.let { diagram.nodes.indices.toSet() - diagram.descendants(it) } ?: emptySet()) }
    val large = diagram.nodes.size > 12 || diagram.edges.size > 24 || diagram.width > 480 || diagram.height > 400
    Column(Modifier.widthIn(min = 200.dp, max = 280.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        RichMessageText(diagram.title, style = MaterialTheme.typography.titleMedium)
        if (diagram.kind == "timeline") diagram.entries.take(2).forEach { entry -> Column { RichMessageText(entry.date, style = MaterialTheme.typography.labelMedium); RichMessageText(entry.label, Modifier.heightIn(max = 72.dp).clipToBounds(), MaterialTheme.typography.bodyMedium) } }
        else if (!large) DiagramPlot(diagram, emptySet(), null, { focused = it; expanded = true }, Modifier.fillMaxWidth().height(220.dp), false)
        else {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) { Glyph("schema", 32); Text("${diagram.nodes.size} nodes · ${diagram.edges.size} connections", style = MaterialTheme.typography.bodyMedium) }
            diagram.nodes.take(3).forEach { RichMessageText(it.label, Modifier.heightIn(max = 56.dp).clipToBounds(), MaterialTheme.typography.bodyMedium) }
        }
        SigilTextButton({ expanded = true }) { Glyph("open_in_full", 18); Spacer(Modifier.width(8.dp)); Text("Open diagram") }
    }
    if (expanded) Dialog({ expanded = false }, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface) {
                Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        SigilIconButton({ expanded = false }) { Glyph("close", 24, "Close diagram") }
                        Text(diagram.kind.replaceFirstChar { it.uppercase() }, Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
                        if (diagram.kind != "timeline") SigilIconButton({ outline = !outline }) { Glyph(if (outline) "schema" else "format_list_bulleted", 24, if (outline) "Show diagram" else "List nodes") }
                    }
                    RichMessageText(diagram.title, Modifier.heightIn(max = 96.dp).verticalScroll(rememberScrollState()), MaterialTheme.typography.titleMedium)
                    if (diagram.kind == "timeline") LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                        itemsIndexed(diagram.entries) { index, entry -> Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Column(Modifier.width(88.dp)) { Text("${index + 1}", style = MaterialTheme.typography.labelSmall); RichMessageText(entry.date, style = MaterialTheme.typography.labelLarge) }
                            RichMessageText(entry.label, Modifier.weight(1f))
                        } }
                    } else {
                        if (outline) LazyColumn(Modifier.weight(1f).fillMaxWidth()) {
                            itemsIndexed(diagram.nodes) { index,node ->
                                Row(Modifier.fillMaxWidth().heightIn(min=48.dp).combinedClickable(onClick={ focused=index;outline=false;collapsed=emptySet();branch=null },onLongClickLabel="Focus node",onLongClick={ focused=index;outline=false;collapsed=emptySet();branch=null }).padding(12.dp),horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                                    Text("${index+1}",style=MaterialTheme.typography.labelLarge)
                                    RichMessageText(node.label,Modifier.weight(1f))
                                }
                            }
                        } else DiagramPlot(diagram, hidden, focused, { focused = it }, Modifier.weight(1f).fillMaxWidth(), true)
                        if (branch != null) SigilTextButton({ branch = null }) { Text("Show whole diagram") }
                        focused?.let { index ->
                            Column(Modifier.fillMaxWidth().heightIn(max = 260.dp).verticalScroll(rememberScrollState()).background(MaterialTheme.colorScheme.surfaceVariant, MaterialTheme.shapes.large).padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                Row(verticalAlignment = Alignment.CenterVertically) {
                                    Column(Modifier.weight(1f)) { RichMessageText(diagram.nodes[index].label, style = MaterialTheme.typography.titleMedium) }
                                    SigilIconButton({ focused = null }) { Glyph("close", 20, "Clear node focus") }
                                }
                                if (diagram.kind == "mindmap" && diagram.edges.any { it.from == index }) SigilTextButton({ collapsed = if (index in collapsed) collapsed - index else collapsed + index }) { Text(if (index in collapsed) "Expand branch" else "Collapse branch") }
                                if (diagram.kind == "org" && diagram.edges.any { it.from == index }) SigilTextButton({ branch = index }) { Text("Focus branch") }
                                diagram.edges.filter { it.from == index || it.to == index }.forEach { edge ->
                                    val target = if (edge.from == index) edge.to else edge.from
                                    Row(Modifier.fillMaxWidth().clickable(role = Role.Button) { focused = target }.padding(vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                        Glyph(if (edge.from == index) "arrow_forward" else "arrow_back", 20, if (edge.from == index) "Connects to" else "Connected from")
                                        Column(Modifier.weight(1f)) { RichMessageText(diagram.nodes[target].label); if (edge.label.text.isNotBlank()) RichMessageText(edge.label, style = MaterialTheme.typography.bodySmall) }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun DiagramPlot(diagram: DiagramContent, hidden: Set<Int>, focused: Int?, focus: (Int) -> Unit, modifier: Modifier, interactive: Boolean) {
    var bounds by remember { mutableStateOf(IntSize.Zero) }
    var zoom by remember(diagram) { mutableFloatStateOf(1f) }
    var pan by remember(diagram) { mutableStateOf(Offset.Zero) }
    var initialized by remember(diagram) { mutableStateOf(false) }
    val density = LocalDensity.current.density
    val ink = LocalContentColor.current
    val surface = LocalMessageSurface.current.takeOrElse { MaterialTheme.colorScheme.surface }
    val connected = remember(diagram, focused) { focused?.let { node -> setOf(node) + diagram.edges.filter { it.from == node || it.to == node }.flatMap { listOf(it.from,it.to) } } ?: diagram.nodes.indices.toSet() }
    fun minimumZoom() = min(bounds.width / (diagram.width * density), bounds.height / (diagram.height * density)).coerceIn(.0001f, 1f)
    fun fit() {
        if (bounds.width == 0 || bounds.height == 0) return
        zoom = if (interactive) minimumZoom() else minimumZoom().coerceAtLeast(.65f)
        pan = Offset(max(0f,(bounds.width-diagram.width*density*zoom)/2),max(0f,(bounds.height-diagram.height*density*zoom)/2))
    }
    LaunchedEffect(diagram, bounds) { if (!initialized && bounds.width > 0 && bounds.height > 0) { fit(); initialized=true } }
    LaunchedEffect(focused, initialized) {
        if (interactive && initialized && focused != null) {
            val node=diagram.nodes[focused]
            zoom=max(zoom,.65f)
            pan=Offset(bounds.width/2f,bounds.height/2f)-Offset((node.x+80)*density*zoom,(node.y+36)*density*zoom)
        }
    }
    fun at(x: Float,y: Float) = Offset(x*density*zoom,y*density*zoom)+pan
    fun shown(rect: Rect) = rect.right > 0 && rect.bottom > 0 && rect.left < bounds.width && rect.top < bounds.height
    Column(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(Modifier.weight(1f).fillMaxWidth().clipToBounds().onSizeChanged { bounds=it }.then(if (!interactive) Modifier else Modifier.pointerInput(diagram, hidden) {
            detectTapGestures { position ->
                if (zoom < .45f) {
                    val node = diagram.nodes.indices.filter { it !in hidden }.minByOrNull { (at(diagram.nodes[it].x+80,diagram.nodes[it].y+36)-position).getDistanceSquared() }
                    if (node != null && (at(diagram.nodes[node].x+80,diagram.nodes[node].y+36)-position).getDistance() < 24.dp.toPx()) focus(node)
                }
            }
        }.pointerInput(diagram) {
            detectTransformGestures { centroid, change, scale, _ ->
                val next = (zoom*scale).coerceIn(minimumZoom(),3f)
                pan=(pan-centroid)*(next/zoom)+centroid+change
                zoom=next
                pan=Offset(pan.x.coerceIn(-diagram.width*density*zoom, size.width.toFloat()),pan.y.coerceIn(-diagram.height*density*zoom,size.height.toFloat()))
            }
        }).semantics { contentDescription="${diagram.kind} diagram viewport" }) {
            Canvas(Modifier.fillMaxSize()) {
                if (zoom < .45f) diagram.nodes.forEachIndexed { index,node -> if (index !in hidden) drawRoundRect(ink.copy(alpha=if(index in connected).7f else .2f),at(node.x,node.y),Size(160*density*zoom,72*density*zoom),CornerRadius(3.dp.toPx()*zoom)) }
                if (diagram.kind == "sequence") diagram.nodes.forEachIndexed { index,node -> if (index !in hidden) drawLine(ink.copy(alpha=.25f),at(node.x+80,node.y+72),at(node.x+80,diagram.height-24),1.dp.toPx(),pathEffect=PathEffect.dashPathEffect(floatArrayOf(6.dp.toPx(),4.dp.toPx()))) }
                diagram.edges.forEach { edge ->
                    if (edge.from in hidden || edge.to in hidden) return@forEach
                    val from=diagram.nodes[edge.from]; val to=diagram.nodes[edge.to]
                    val strong=focused==null || edge.from==focused || edge.to==focused
                    val color=ink.copy(alpha=if(strong) .8f else .16f)
                    val start:Offset; val end:Offset; val control1:Offset; val control2:Offset
                    if(diagram.kind=="sequence") {
                        start=at(from.x+80,edge.y); end=at(to.x+80,edge.y)
                        if(edge.from==edge.to) { control1=start+Offset(48.dp.toPx()*zoom,0f);control2=end+Offset(48.dp.toPx()*zoom,32.dp.toPx()*zoom) } else { control1=start;control2=end }
                    } else if(diagram.kind=="mindmap") {
                        val a=Offset(from.x+80,from.y+36);val b=Offset(to.x+80,to.y+36);val delta=b-a
                        val border=min(if(abs(delta.x)<.001f)Float.MAX_VALUE else 80/abs(delta.x),if(abs(delta.y)<.001f)Float.MAX_VALUE else 36/abs(delta.y))
                        start=at((a+delta*border).x,(a+delta*border).y);end=at((b-delta*border).x,(b-delta*border).y);control1=start;control2=end
                    } else if(to.y>from.y) {
                        start=at(from.x+80,from.y+72);end=at(to.x+80,to.y);control1=Offset(start.x,(start.y+end.y)/2);control2=Offset(end.x,control1.y)
                    } else {
                        start=at(from.x+160,from.y+36);end=at(to.x+160,to.y+36);val side=max(start.x,end.x)+48.dp.toPx()*zoom;control1=Offset(side,start.y-36.dp.toPx()*zoom);control2=Offset(side,end.y+36.dp.toPx()*zoom)
                    }
                    val path=Path().apply { moveTo(start.x,start.y);cubicTo(control1.x,control1.y,control2.x,control2.y,end.x,end.y) }
                    drawPath(path,color,style=Stroke(1.5.dp.toPx(),pathEffect=if(edge.dashed)PathEffect.dashPathEffect(floatArrayOf(6.dp.toPx(),4.dp.toPx()))else null))
                    val direction=end-(if(control2==end)start else control2);val angle=atan2(direction.y,direction.x);val size=7.dp.toPx()
                    drawLine(color,end,end-Offset(cos(angle-.5f),sin(angle-.5f))*size,1.5.dp.toPx());drawLine(color,end,end-Offset(cos(angle+.5f),sin(angle+.5f))*size,1.5.dp.toPx())
                }
            }
            diagram.edges.forEachIndexed { index, edge ->
                if(zoom < .45f || edge.from in hidden || edge.to in hidden || edge.label.text.isBlank()) return@forEachIndexed
                val from=diagram.nodes[edge.from];val to=diagram.nodes[edge.to]
                val width=if(diagram.kind=="sequence")max(96f,abs(from.x-to.x)) else 128f
                val labelAt=when {
                    diagram.kind=="sequence" -> at(min(from.x,to.x)+80,edge.y-40)
                    diagram.kind=="mindmap" -> at((from.x+to.x)/2+80-width/2,(from.y+to.y)/2+20)
                    to.y>from.y -> at((from.x+to.x)/2+80-width/2,(from.y+72+to.y)/2-20)
                    else -> at(max(from.x,to.x)+168,(from.y+to.y)/2+16)
                }
                if(shown(Rect(labelAt,Size(width*density*zoom,40*density*zoom)))) Box(Modifier.offset { IntOffset(labelAt.x.roundToInt(),labelAt.y.roundToInt()) }.width((width*zoom).dp).heightIn(max=(44*zoom).dp).clipToBounds().background(surface)
                    .clickable(role=Role.Button) { focus(edge.from) }.semantics { contentDescription="Connection ${index+1}" }) { RichMessageText(edge.label,style=MaterialTheme.typography.labelSmall.copy(fontSize=(12*zoom).coerceAtLeast(11f).sp)) }
            }
            diagram.nodes.forEachIndexed { index,node ->
                if(zoom < .45f || index in hidden) return@forEachIndexed
                val position=at(node.x,node.y)
                if(!shown(Rect(position,Size(160*density*zoom,72*density*zoom)))) return@forEachIndexed
                val shape=when { node.shape=="decision" -> GenericShape { size,_ -> moveTo(size.width/2,0f);lineTo(size.width,size.height/2);lineTo(size.width/2,size.height);lineTo(0f,size.height/2);close() }; node.shape=="process" -> RoundedCornerShape(4.dp); else -> RoundedCornerShape(16.dp) }
                Surface(Modifier.offset { IntOffset(position.x.roundToInt(),position.y.roundToInt()) }.size((160*zoom).dp,(72*zoom).dp).alpha(if(index in connected)1f else .3f)
                    .combinedClickable(onClick={focus(index)},onLongClickLabel="Focus node",onLongClick={focus(index)}).semantics { contentDescription="Node ${index+1}"; selected=focused==index }, shape=shape,
                    color=MaterialTheme.colorScheme.surfaceVariant,contentColor=MaterialTheme.colorScheme.onSurfaceVariant,border=BorderStroke(if(focused==index)2.dp else 1.dp,ink.copy(alpha=if(focused==index).8f else .2f))) {
                    CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surfaceVariant) {
                        Box(Modifier.padding(horizontal=if(node.shape=="decision")28.dp else 10.dp,vertical=8.dp).clipToBounds(),contentAlignment=Alignment.Center) { RichMessageText(node.label,style=MaterialTheme.typography.bodyMedium.copy(fontSize=(16*zoom).coerceAtLeast(12f).sp)) }
                    }
                }
            }
        }
        if(interactive) SigilTextButton({fit()}) { Text("Fit diagram") }
        if(interactive && zoom < .45f) Text("Overview · zoom in to read labels", style=MaterialTheme.typography.labelSmall)
    }
}
