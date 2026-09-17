package org.sigil

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

internal fun MessagePart.previewLeaves(): List<MessagePart> = if (kind == "composition") previewParts.flatMap { it.previewLeaves() } else listOf(this)
private fun MessagePart.previewEffects(): List<String> = buildList {
    rich?.let(::add); addAll(items.mapNotNull { it.rich })
    table?.let { addAll(it.columns); it.rows.forEach(::addAll) }
    recipe?.let { add(it.title); addAll(it.ingredients); addAll(it.steps) }
    chart?.let { add(it.title); addAll(it.points.map { point -> point.label }) }
    diagram?.let { add(it.title); addAll(it.nodes.map { node -> node.label }); addAll(it.edges.map { edge -> edge.label }); it.entries.forEach { entry -> add(entry.date); add(entry.label) } }
    utility?.let { it.rich?.let(::add); it.secondary?.let(::add); addAll(it.details) }
}.flatMap { it.motion }.map { it.kind }

@Composable internal fun TypedSigilPreview(source: String, modifier: Modifier = Modifier, open: ((PreviewIntent, String) -> Unit)? = null) {
    val resolve = LocalStructuredPreview.current
    var resolved by remember { mutableStateOf<MessagePart?>(null) }
    var resolvedSource by remember { mutableStateOf("") }
    LaunchedEffect(source, resolve) {
        if (source.isBlank() || source.length > 16384 || resolve == null) { resolved = null; return@LaunchedEffect }
        delay(120)
        resolved = withContext(Dispatchers.Default) { resolve(source) }
        resolvedSource = source
    }
    DraftPreviewContent(resolved,source,source==resolvedSource,modifier,open)
}

@Composable internal fun StructuredDraftPreview(part:MessagePart,source:String) {
    DraftPreviewContent(part,source,true,Modifier,null)
}

@Composable private fun DraftPreviewContent(resolved:MessagePart?,source:String,current:Boolean,modifier:Modifier,open:((PreviewIntent,String)->Unit)?) {
    val parts = resolved?.previewLeaves().orEmpty()
    val cards = parts.filter { it.kind != "text" }
    val effects = parts.flatMap { it.previewEffects() }.distinct()
    val motion = LocalMotion.current
    val launch = LocalPreviewLaunch.current
    DisposableEffect(launch) { onDispose { launch?.source = ""; launch?.bounds = Rect.Zero; launch?.visibleOrigins?.clear() } }
    SideEffect { launch?.source = source.takeIf {current}.orEmpty() }
    AnimatedVisibility(cards.isNotEmpty() || effects.isNotEmpty(), modifier,
        enter = expandVertically(motion.enter(MotionMillis)) + fadeIn(motion.enter(MotionMillis)),
        exit = shrinkVertically(motion.exit(MotionMillis)) + fadeOut(motion.exit(MotionExit)), label = "Draft preview") {
        Column(Modifier.fillMaxWidth().heightIn(max = 280.dp).verticalScroll(rememberScrollState()).testTag("typed-sigil-preview"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            CompositionLocalProvider(LocalTextMotion provides null) {
                cards.forEachIndexed { index,part ->
                    if (part.previewIntent != null) IntentPreview(part.previewIntent, open?.takeIf {current}?.let {action->{action(part.previewIntent,source)}})
                    else if (part.randomizerPreview != null) UnresolvedRandomizer(part,cards.take(index).count {it.randomizerPreview!=null})
                    else BuilderPreview(part)
                }
            }
            if (effects.isNotEmpty()) Row(Modifier.padding(horizontal = 12.dp, vertical = 4.dp).clearAndSetSemantics { contentDescription = "Animated text: ${effects.joinToString()}" },
                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Glyph("motion_photos_on", 18)
                Text(effects.joinToString(" · ") { it.replaceFirstChar(Char::uppercase) }, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable private fun IntentPreview(intent: PreviewIntent, open: (() -> Unit)?) {
    Column(Modifier.fillMaxWidth().padding(12.dp), verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {Glyph("pending",20);Text("Draft",style=MaterialTheme.typography.labelMedium)}
        Text(intent.tool,style=MaterialTheme.typography.titleMedium)
        Text(when { intent.tool=="Contact QR" -> "Resolve this account in your contacts before creating its QR code."; !intent.ready -> "Complete the details in ${intent.tool.lowercase()}."; intent.tool=="Contact" -> "Choose and confirm this account before sharing."; intent.tool=="Weather" -> "Choose a place and provider before lookup."; else -> "Choose a provider before lookup." },style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
        if(open!=null && intent.tool in listOf("Translation","Definition","Weather","Contact"))SigilTextButton(open) {Glyph(if(intent.tool=="Contact")"person_search" else "arrow_forward",18);Spacer(Modifier.width(8.dp));Text("Set up ${intent.tool.lowercase()}")}
    }
}

@Composable private fun UnresolvedRandomizer(part: MessagePart,ordinal:Int) {
    val preview = part.randomizerPreview ?: return
    val objects = LocalSolidMaterial.current != null
    val launch = LocalPreviewLaunch.current
    DisposableEffect(launch,ordinal) {onDispose {launch?.visibleOrigins?.remove(ordinal)}}
    Column(Modifier.fillMaxWidth().padding(vertical=8.dp).semantics { contentDescription = "Result chosen on send." }, horizontalAlignment = Alignment.CenterHorizontally) {
        BoxWithConstraints(Modifier.fillMaxWidth(),contentAlignment=Alignment.Center) {
            val count=if(preview.kind=="dice")preview.sides.take(6).size else 1
            val columns=minOf(3,count).coerceAtLeast(1)
            val side=when(preview.kind){"dice"->minOf(MaterialDiceUnit*MaterialObjectScale,(maxWidth-4.dp*(columns-1))/columns);"coin"->minOf(MaterialCoinUnit*MaterialObjectScale,maxWidth);else->minOf(MaterialCardWidth,maxWidth)}
            Column(Modifier.onGloballyPositioned {
                val bounds=Rect(it.localToWindow(Offset.Zero),Size(it.size.width.toFloat(),it.size.height.toFloat()))
                if(objects && preview.kind in listOf("dice","coin","cards") && (preview.kind!="dice" || preview.sides.take(6).all {side->side in listOf(4,6,8,10,12,16,20,24,30,100)})) {
                    launch?.visibleOrigins?.set(ordinal,bounds);launch?.bounds=bounds
                }
            }.testTag("randomizer-preview-objects"),verticalArrangement=Arrangement.spacedBy(4.dp),horizontalAlignment=Alignment.CenterHorizontally) {
                if(launch?.lifted(ordinal)==true)Spacer(Modifier.size(side*columns+4.dp*(columns-1),if(preview.kind=="cards")side*1.47f else side*((count+columns-1)/columns)+4.dp*((count+columns-1)/columns-1)))
                else if(objects && preview.kind!="number") {
                    val platform=LocalMaterialPlatform.current
                    if(preview.kind=="dice")preview.sides.take(6).chunked(columns).forEach {row->
                        Row(horizontalArrangement=Arrangement.spacedBy(4.dp)) {row.forEach {sides->
                            if(sides in listOf(4,6,8,10,12,16,20,24,30,100))platform.Object(0,if(sides==100)10 else sides,1,null,null,Modifier.size(side),1f)
                            else Box(Modifier.size(side),contentAlignment=Alignment.Center) {Glyph("casino",48)}
                        }}
                    } else platform.Object(if(preview.kind=="coin")1 else 2,0,0,if(preview.kind=="cards")floatArrayOf(0f,1f,0f,0f)else null,"",Modifier.size(side,if(preview.kind=="cards")side*1.47f else side),1f)
                } else Box(Modifier.size(if(preview.kind=="number")48.dp else side),contentAlignment=Alignment.Center) {Glyph(when(preview.kind){"dice"->"casino";"coin"->"toll";"cards"->"style";else->"numbers"},48)}
            }
        }
        Text(part.text,style=MaterialTheme.typography.labelMedium,color=MaterialTheme.colorScheme.onSurfaceVariant,maxLines=2,overflow=TextOverflow.Ellipsis)
    }
}
