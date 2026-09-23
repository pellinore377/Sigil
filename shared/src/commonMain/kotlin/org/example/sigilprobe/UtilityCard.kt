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
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*
import kotlin.math.floor

val LocalSensitiveCopy = staticCompositionLocalOf<((String) -> Unit)?> { null }
val LocalMathContent = staticCompositionLocalOf<(@Composable (String, String, Modifier) -> Unit)?> { null }

internal fun utilityLabel(value: UtilityContent) = when (value.kind) {
    "calculation" -> "Calculation"; "conversion" -> "Conversion"; "math" -> "Formula"; "qr" -> when (value.qr?.kind) {
        "wifi" -> "Wi-Fi QR code"; "contact" -> "Contact QR code"; else -> "QR code"
    }
    "dice" -> "Dice"; "pick" -> if(value.motion?.kind=="coin")"Coin flip" else "Choice"; "random" -> "Random number"; "swatch" -> "Color"
    "keys" -> "Keyboard shortcut"; "rating" -> "Rating"; "progress" -> "Progress"; "quote" -> "Quote"; else -> "Details"
}
internal fun utilityAction(value: UtilityContent) = when (value.kind) { "qr" -> "QR code"; "math" -> "formula"; else -> utilityLabel(value).lowercase() }

@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
internal fun UtilityCard(value: UtilityContent) {
    if (value.kind == "art") { AsciiArt(RichText(value.display)); return }
    if (value.kind == "progress") { ProgressCard(value); return }
    if (value.kind == "rating") { RatingCard(value); return }
    if (value.kind == "random") { NumberPickCard(value); return }
    when (value.kind) { "calculation" -> { CalculationCard(value); return }; "conversion" -> { ConversionCard(value); return }; "math" -> { MathCard(value); return } }
    val clipboard = LocalClipboardManager.current
    val clock=LocalTextMotion.current?.clock
    val motion=LocalMotion.current
    val animate=LocalAppearance.current.messageEffects && !motion.reduced
    val objectMessage=value.motion?.kind in listOf("dice","coin","choice")
    // These carry a figure, not a block: the bubble must shrink to it instead of holding a 200.dp floor.
    val hug = value.kind in listOf("rating","qr","swatch","keys")
    fun resultAlpha()=if(animate && value.motion!=null && (clock?.elapsed ?: 12000f)<(clock?.duration(randomizerDuration(value.motion)) ?: randomizerDuration(value.motion)))0f else 1f
    val label = utilityLabel(value)
    val action = utilityAction(value)
    val icon = when (value.kind) {
        "qr" -> "qr_code"
        "dice" -> "casino"; "pick" -> if (value.motion?.kind == "coin") "toll" else "playing_cards"; "random" -> "numbers"; "swatch" -> "palette"
        "keys" -> "keyboard"; "rating" -> "star"; "progress" -> "data_usage"; "quote" -> "format_quote"; else -> "data_object"
    }
    @Composable fun body() {
        if(value.qr != null) { QrBody(value, value.qr, false); return }
        if(value.kind == "quote") { QuoteBody(value); return }
        if(value.kind == "keys") { KeysBody(value); return }
        if(objectMessage) {
            RandomizerStage(value.motion!!,false,value.rich,if(value.motion.kind=="choice")choiceDescription(value) else null)
            RandomizerCaption(value,resultAlpha()>0f)
            value.secondary?.let {RichMessageText(it,style=MaterialTheme.typography.bodyMedium)}
            return
        }
        value.motion?.let {RandomizerStage(it,false,value.rich)}
        if(value.kind=="dice" && value.details.size>6)Text("6 of ${value.details.size} dice shown",style=MaterialTheme.typography.labelSmall)
        if (value.kind == "swatch") {
            val rgba = value.rgba ?: 0L
            val color = Color((rgba shr 24 and 255).toInt(), (rgba shr 16 and 255).toInt(), (rgba shr 8 and 255).toInt(), (rgba and 255).toInt())
            val ink = LocalContentColor.current
            val chip = with(LocalDensity.current) { MaterialTheme.typography.bodyLarge.lineHeight.toDp() }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Canvas(Modifier.size(chip * 1.6f, chip).clip(RoundedCornerShape(5.dp)).semantics { contentDescription = "Color sample ${value.display}" }) {
                    // The checker only earns its place behind a translucent colour; otherwise it shows as a fringe.
                    if ((rgba and 255L) < 255L) {
                        val side = size.height / 2
                        for (y in 0..(size.height / side).toInt()) for (x in 0..(size.width / side).toInt())
                            drawRect(ink.copy(alpha = if ((x + y) % 2 == 0) .06f else .16f), androidx.compose.ui.geometry.Offset(x * side, y * side), androidx.compose.ui.geometry.Size(side, side))
                    }
                    drawRect(color)
                }
                Text(value.display, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        value.rich?.let { RichMessageText(it,Modifier.graphicsLayer {alpha=resultAlpha()},MaterialTheme.typography.bodyLarge) }
        if (value.kind !in listOf("rating","swatch") && value.display.isNotEmpty() && !(value.kind=="random" && value.motion!=null) && value.motion?.kind!="coin") Text(value.display, modifier=Modifier.graphicsLayer {alpha=resultAlpha()},
            style = if (value.kind == "random") MaterialTheme.typography.headlineMedium.copy(fontFamily = LocalCodeFont.current) else MaterialTheme.typography.bodyLarge,
            maxLines = 3, overflow = TextOverflow.Ellipsis)
        if (value.alternate.isNotEmpty()) Text(value.alternate, style = MaterialTheme.typography.bodyLarge, maxLines = 3, overflow = TextOverflow.Ellipsis)
        if (value.kind !in listOf("rating","progress")) value.ratio?.let { ratio ->
            val ink = LocalContentColor.current
            Canvas(Modifier.fillMaxWidth().height(8.dp).semantics { progressBarRangeInfo = ProgressBarRangeInfo(ratio.coerceIn(0f, 1f), 0f..1f) }) {
                drawRoundRect(ink.copy(alpha = .15f), cornerRadius = androidx.compose.ui.geometry.CornerRadius(size.height / 2))
                drawRoundRect(ink, size = size.copy(width = size.width * ratio.coerceIn(0f, 1f)), cornerRadius = androidx.compose.ui.geometry.CornerRadius(size.height / 2))
            }
        }
        value.secondary?.let { RichMessageText(it, style = MaterialTheme.typography.bodyMedium) }
    }
    Column(Modifier.widthIn(min = if (hug) Dp.Unspecified else 200.dp, max = 280.dp).animateContentSize(motion.tween(MotionMillis)), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if(!objectMessage)Row(if (hug) Modifier else Modifier.fillMaxWidth(), verticalAlignment=Alignment.CenterVertically, horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            Glyph(icon, 20)
            Text(label, if (hug) Modifier else Modifier.weight(1f), style = MaterialTheme.typography.labelMedium)
            value.copy?.let { copy -> SigilIconButton({clipboard.setText(AnnotatedString(copy))}) {Glyph("content_copy",24,"Copy $action")} }
        }
        body()
    }
}

@Composable
private fun QuoteBody(value: UtilityContent) {
    val ink = LocalContentColor.current
    val mark = MaterialTheme.typography.headlineMedium
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Text("“", Modifier.align(Alignment.Top), style = mark, color = ink.copy(alpha = .32f))
            value.rich?.let { RichMessageText(it, Modifier.weight(1f), MaterialTheme.typography.titleMedium) }
            Text("”", Modifier.align(Alignment.Bottom), style = mark, color = ink.copy(alpha = .32f))
        }
        val source = value.details.firstOrNull()
        if (value.secondary != null || source != null) Row(Modifier.align(Alignment.End), verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Text("—", style = MaterialTheme.typography.bodyMedium, color = ink.copy(alpha = .7f))
            value.secondary?.let { RichMessageText(it, style = MaterialTheme.typography.bodyMedium) }
            source?.let {
                Text("·", style = MaterialTheme.typography.bodyMedium, color = ink.copy(alpha = .7f))
                CompositionLocalProvider(LocalContentColor provides ink.copy(alpha = .7f)) { RichMessageText(it, style = MaterialTheme.typography.bodyMedium) }
            }
        }
    }
}

@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
private fun KeysBody(value: UtilityContent) {
    val ink = LocalContentColor.current
    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        value.details.forEachIndexed { index, detail ->
            if (index > 0) Text("+", Modifier.align(Alignment.CenterVertically), style = MaterialTheme.typography.labelMedium, color = ink.copy(alpha = .6f))
            Keycap(detail)
        }
    }
}

// Two stacked alphas of the bubble ink: a lit face over a dimmer rim, so the cap reads without elevation or a second scheme role.
@Composable
private fun Keycap(detail: RichText) {
    val ink = LocalContentColor.current
    val face = with(LocalDensity.current) { MaterialTheme.typography.bodyMedium.lineHeight.toDp() }
    Box(Modifier.clip(RoundedCornerShape(8.dp)).background(ink.copy(alpha = .14f)).padding(bottom = 3.dp)) {
        Box(Modifier.clip(RoundedCornerShape(7.dp)).background(ink.copy(alpha = .16f)).heightIn(min = face + 6.dp).widthIn(min = 26.dp)
            .padding(horizontal = 8.dp, vertical = 2.dp), contentAlignment = Alignment.Center) {
            RichMessageText(detail, style = MaterialTheme.typography.bodyMedium.copy(fontFamily = LocalCodeFont.current))
        }
    }
}

@Composable
private fun QrBody(value: UtilityContent, qr: QrContent, full: Boolean) {
    var revealed by remember(value) { mutableStateOf(false) }
    var password by remember(value) { mutableStateOf(false) }
    val clipboard = LocalClipboardManager.current
    val uri = LocalUriHandler.current
    val sensitiveCopy = LocalSensitiveCopy.current
    val visible = !qr.concealed || revealed
    if (visible) QrImage(qr, if (full) 480.dp else 232.dp)
    else SigilTextButton({ revealed = true }) { Glyph("visibility", 20); Spacer(Modifier.width(8.dp)); Text("Reveal QR code") }
    // The type indicator already names the kind; a fixed "Text QR code" label under the tile would only repeat it.
    if (qr.kind != "text") value.rich?.let { RichMessageText(it, style = MaterialTheme.typography.labelSmall) }
    if (visible) {
        if (qr.kind == "url") SigilTextButton({ uri.openUri(qr.payload) }) { Glyph("open_in_new", 18); Spacer(Modifier.width(8.dp)); Text("Open link") }
        SigilTextButton({ if (qr.kind == "wifi" && sensitiveCopy != null) sensitiveCopy(qr.payload) else clipboard.setText(AnnotatedString(qr.payload)) }) { Glyph("content_copy", 18); Spacer(Modifier.width(8.dp)); Text(if (qr.kind == "wifi") "Copy Wi-Fi details" else "Copy QR contents") }
    }
    if (full && visible) when (qr.kind) {
        "wifi" -> {
            Text("Scanning shares access to this network.", style = MaterialTheme.typography.bodyMedium)
            SigilTextButton({ password = !password }) { Glyph(if (password) "expand_less" else "expand_more", 18); Spacer(Modifier.width(8.dp)); Text(if (password) "Hide password" else "Show password") }
            if (password) qr.password?.let { RichMessageText(it) }
        }
        "contact" -> Text("This code contains a contact address and key. It does not approve a device.", style = MaterialTheme.typography.bodyMedium)
    }
}

@Composable
internal fun QrDetails(value: UtilityContent, dismiss: () -> Unit) {
    val qr = value.qr ?: return
    val clipboard = LocalClipboardManager.current
    val action = utilityAction(value)
    Dialog(dismiss, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface) {
                Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        SigilIconButton(dismiss) { Glyph("close", 24, "Close $action") }
                        Text(utilityLabel(value), Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
                        value.copy?.let { copy -> SigilIconButton({ clipboard.setText(AnnotatedString(copy)) }) { Glyph("content_copy", 24, "Copy $action") } }
                    }
                    Column(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) { QrBody(value, qr, true) }
                }
            }
        }
    }
}

// Snapped to a whole number of modules: a fractional remainder would pad the tile with unscannable white.
@Composable
private fun QrImage(qr: QrContent, cap: Dp) {
    val density = LocalDensity.current
    val side = remember(qr.width, cap, density.density) {
        if (qr.width !in 1..185) cap
        else with(density) { (floor(cap.toPx() / qr.width).coerceAtLeast(1f) * qr.width).toDp() }
    }
    Canvas(Modifier.size(side).background(Color.White).semantics { contentDescription = "Scannable QR code" }) {
        if (qr.width !in 1..185) return@Canvas
        val module = floor(minOf(size.width, size.height) / qr.width)
        if (module < 1 || qr.cells.length != qr.width * qr.width) return@Canvas
        val origin = androidx.compose.ui.geometry.Offset(floor((size.width - module * qr.width) / 2), floor((size.height - module * qr.width) / 2))
        qr.cells.forEachIndexed { i, cell -> if (cell == '1') drawRect(Color.Black,
            origin + androidx.compose.ui.geometry.Offset(i % qr.width * module, i / qr.width * module), androidx.compose.ui.geometry.Size(module, module)) }
    }
}
