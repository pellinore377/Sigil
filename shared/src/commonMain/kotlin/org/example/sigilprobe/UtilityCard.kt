package org.sigil

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*
import kotlin.math.floor

val LocalSensitiveCopy = staticCompositionLocalOf<((String) -> Unit)?> { null }
val LocalMathContent = staticCompositionLocalOf<(@Composable (String, String, Modifier) -> Unit)?> { null }

@Composable
internal fun UtilityCard(value: UtilityContent) {
    if (value.kind == "art") { CodeBlock(RichText(value.display), "ASCII art"); return }
    var expanded by remember(value) { mutableStateOf(false) }
    var revealed by remember(value) { mutableStateOf(false) }
    var password by remember(value) { mutableStateOf(false) }
    var swapped by remember(value) { mutableStateOf(false) }
    var showAll by remember(value) { mutableStateOf(false) }
    val clipboard = LocalClipboardManager.current
    val uri = LocalUriHandler.current
    val sensitiveCopy = LocalSensitiveCopy.current
    val qr = value.qr
    val visible = qr?.concealed != true || revealed
    val clock=LocalTextMotion.current?.clock
    val motion=LocalMotion.current
    val animate=LocalAppearance.current.messageEffects && !motion.reduced
    val objectMessage=value.motion?.kind in listOf("dice","coin","choice")
    val canExpand=qr!=null || value.kind=="math"
    fun resultAlpha(full:Boolean)=if(!full && animate && value.motion!=null && (clock?.elapsed ?: 12000f)<(clock?.duration(randomizerDuration(value.motion)) ?: randomizerDuration(value.motion)))0f else 1f
    val label = when (value.kind) {
        "calculation" -> "Calculation"; "conversion" -> "Conversion"; "math" -> "Formula"; "qr" -> when (qr?.kind) {
            "wifi" -> "Wi-Fi QR code"; "contact" -> "Contact QR code"; else -> "QR code"
        }
        "dice" -> "Dice"; "pick" -> if(value.motion?.kind=="coin")"Coin flip" else "Choice"; "random" -> "Random number"; "swatch" -> "Color"
        "keys" -> "Keyboard shortcut"; "rating" -> "Rating"; "progress" -> "Progress"; "quote" -> "Quote"; else -> "Details"
    }
    val action = when (value.kind) { "qr" -> "QR code"; "math" -> "formula"; else -> label.lowercase() }
    val icon = when (value.kind) {
        "calculation" -> "calculate"; "conversion" -> "swap_horiz"; "math" -> "functions"; "qr" -> "qr_code"
        "dice" -> "casino"; "pick" -> if (value.motion?.kind == "coin") "toll" else "playing_cards"; "random" -> "numbers"; "swatch" -> "palette"
        "keys" -> "keyboard"; "rating" -> "star"; "progress" -> "data_usage"; "quote" -> "format_quote"; else -> "data_object"
    }
    @Composable fun body(full: Boolean) {
        if(objectMessage) {
            RandomizerStage(value.motion!!,full,value.rich)
            if(value.motion.kind=="dice" && value.motion.dice.size>1) {
                val alpha by androidx.compose.animation.core.animateFloatAsState(resultAlpha(full),motion.tween(MotionMillis),label="Dice total")
                Text(if(value.motion.result.isNotEmpty())"Total · ${value.motion.result}" else value.display,Modifier.fillMaxWidth().graphicsLayer {this.alpha=alpha},textAlign=androidx.compose.ui.text.style.TextAlign.End,style=MaterialTheme.typography.labelLarge)
            }
            if(value.details.size>6)Text("6 of ${value.details.size} dice shown",style=MaterialTheme.typography.labelSmall)
            value.secondary?.let {RichMessageText(it,style=MaterialTheme.typography.bodyMedium)}
            return
        }
        if (qr != null) {
            value.rich?.let { RichMessageText(it) }
            if (visible) QrImage(qr, Modifier.widthIn(max = if (full) 480.dp else 232.dp).fillMaxWidth().aspectRatio(1f))
            else SigilTextButton({ revealed = true }) { Glyph("visibility", 20); Spacer(Modifier.width(8.dp)); Text("Reveal QR code") }
            if (full && visible) {
                when (qr.kind) {
                    "wifi" -> {
                        Text("Scanning shares access to this network.", style = MaterialTheme.typography.bodyMedium)
                        SigilTextButton({ password = !password }) { Glyph(if (password) "expand_less" else "expand_more", 18); Spacer(Modifier.width(8.dp)); Text(if (password) "Hide password" else "Show password") }
                        if (password) qr.password?.let { RichMessageText(it) }
                    }
                    "contact" -> Text("This code contains a contact address and key. It does not approve a device.", style = MaterialTheme.typography.bodyMedium)
                    else -> if (qr.kind == "url") SigilTextButton({ uri.openUri(qr.payload) }) { Glyph("open_in_new", 18); Spacer(Modifier.width(8.dp)); Text("Open link") }
                }
                SigilTextButton({ if (qr.kind == "wifi" && sensitiveCopy != null) sensitiveCopy(qr.payload) else clipboard.setText(AnnotatedString(qr.payload)) }) { Glyph("content_copy", 18); Spacer(Modifier.width(8.dp)); Text(if (qr.kind == "wifi") "Copy Wi-Fi details" else "Copy QR contents") }
            }
        } else if (value.kind == "math") {
            val render = LocalMathContent.current
            if (render != null && value.mathml != null) render(value.mathml, value.display, Modifier.fillMaxWidth().heightIn(max = with(LocalDensity.current) { MaterialTheme.typography.bodyLarge.lineHeight.toDp() * if (full) 12 else 5 }))
            else SelectionContainer { Text(value.display, style = MaterialTheme.typography.bodyMedium.copy(fontFamily = LocalCodeFont.current), maxLines = if (full) Int.MAX_VALUE else 5, overflow = TextOverflow.Ellipsis) }
        } else {
            value.motion?.let {RandomizerStage(it,full,value.rich)}
            if(value.kind=="dice" && value.details.size>6)Text("6 of ${value.details.size} dice shown",style=MaterialTheme.typography.labelSmall)
            if (value.kind == "swatch") {
                val rgba = value.rgba ?: 0L
                val color = Color((rgba shr 24 and 255).toInt(), (rgba shr 16 and 255).toInt(), (rgba shr 8 and 255).toInt(), (rgba and 255).toInt())
                Canvas(Modifier.fillMaxWidth().height(if (full) 180.dp else 64.dp).clip(RoundedCornerShape(12.dp)).semantics { contentDescription = "Color sample ${value.display}" }) {
                    val side = 12.dp.toPx()
                    for (y in 0..(size.height / side).toInt()) for (x in 0..(size.width / side).toInt())
                        drawRect(if ((x + y) % 2 == 0) Color.White else Color.LightGray, androidx.compose.ui.geometry.Offset(x * side, y * side), androidx.compose.ui.geometry.Size(side, side))
                    drawRect(color)
                }
            }
            value.rich?.let { RichMessageText(it,Modifier.graphicsLayer {alpha=resultAlpha(full)}) }
            if (value.display.isNotEmpty() && !(value.kind=="random" && value.motion!=null) && value.motion?.kind!="coin") Text(if (swapped) value.alternate else value.display, modifier=Modifier.graphicsLayer {alpha=resultAlpha(full)},
                style = if (value.kind in listOf("calculation", "random", "rating", "progress")) MaterialTheme.typography.headlineMedium.copy(fontFamily = LocalCodeFont.current) else MaterialTheme.typography.bodyLarge,
                maxLines = if (full) Int.MAX_VALUE else 3, overflow = TextOverflow.Ellipsis)
            if (value.alternate.isNotEmpty()) {
                Text(if (swapped) value.display else value.alternate, style = MaterialTheme.typography.bodyLarge, maxLines = if (full) Int.MAX_VALUE else 3, overflow = TextOverflow.Ellipsis)
                if (value.kind == "conversion") SigilTextButton({ swapped = !swapped }) { Glyph("swap_vert", 20); Spacer(Modifier.width(8.dp)); Text("Swap display") }
            }
            value.ratio?.let { ratio ->
                val ink = LocalContentColor.current
                Canvas(Modifier.fillMaxWidth().height(8.dp).semantics { progressBarRangeInfo = ProgressBarRangeInfo(ratio.coerceIn(0f, 1f), 0f..1f) }) {
                    drawRoundRect(ink.copy(alpha = .15f), cornerRadius = androidx.compose.ui.geometry.CornerRadius(size.height / 2))
                    drawRoundRect(ink, size = size.copy(width = size.width * ratio.coerceIn(0f, 1f)), cornerRadius = androidx.compose.ui.geometry.CornerRadius(size.height / 2))
                }
            }
            value.secondary?.let { RichMessageText(it, style = MaterialTheme.typography.bodyMedium) }
            val capped = value.kind in listOf("keys", "quote")
            val details = if (full) value.details else if (capped) (if (showAll) value.details else value.details.take(5)) else emptyList()
            details.forEachIndexed { index, detail ->
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (value.selected == index) Glyph("check", 20, "Selected choice")
                    if (value.kind == "keys") Surface(shape = MaterialTheme.shapes.small, color = LocalContentColor.current.copy(alpha = .08f)) {
                        RichMessageText(detail, Modifier.padding(10.dp), MaterialTheme.typography.bodyMedium.copy(fontFamily = LocalCodeFont.current))
                    } else RichMessageText(detail)
                }
            }
            if (!full && capped && value.details.size > 5) SigilTextButton({ showAll = !showAll }) { Glyph(if (showAll) "expand_less" else "expand_more", 18); Spacer(Modifier.width(8.dp)); Text(if (showAll) "Show less" else "Show all ${value.details.size}") }
        }
    }
    Column(Modifier.widthIn(min = 200.dp, max = 280.dp).animateContentSize(motion.tween(MotionMillis)), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if(!objectMessage)Row(Modifier.fillMaxWidth(), verticalAlignment=Alignment.CenterVertically, horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            Glyph(icon, 20)
            Text(label, Modifier.weight(1f), style = MaterialTheme.typography.labelMedium)
            if(!canExpand)value.copy?.let { copy -> SigilIconButton({clipboard.setText(AnnotatedString(copy))}) {Glyph("content_copy",24,"Copy $action")} }
        }
        body(false)
        if(canExpand)SigilTextButton({ expanded = true }) { Glyph("open_in_full", 18); Spacer(Modifier.width(8.dp)); Text("Open $action") }
    }
    if (expanded) Dialog({ expanded = false }, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface) {
                Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        SigilIconButton({ expanded = false }) { Glyph("close", 24, "Close $action") }
                        Text(label, Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
                        value.copy?.let { copy -> SigilIconButton({ clipboard.setText(AnnotatedString(copy)) }) { Glyph("content_copy", 24, "Copy $action") } }
                    }
                    Column(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) { body(true) }
                }
            }
        }
    }
}

@Composable
private fun QrImage(qr: QrContent, modifier: Modifier) {
    Canvas(modifier.background(Color.White).semantics { contentDescription = "Scannable QR code" }) {
        if (qr.width !in 1..185) return@Canvas
        val module = floor(minOf(size.width, size.height) / qr.width)
        if (module < 1 || qr.cells.length != qr.width * qr.width) return@Canvas
        val origin = androidx.compose.ui.geometry.Offset(floor((size.width - module * qr.width) / 2), floor((size.height - module * qr.width) / 2))
        qr.cells.forEachIndexed { i, cell -> if (cell == '1') drawRect(Color.Black,
            origin + androidx.compose.ui.geometry.Offset(i % qr.width * module, i / qr.width * module), androidx.compose.ui.geometry.Size(module, module)) }
    }
}
