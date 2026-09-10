package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.AnnotatedString
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
    val clipboard = LocalClipboardManager.current
    val uri = LocalUriHandler.current
    val sensitiveCopy = LocalSensitiveCopy.current
    val qr = value.qr
    val visible = qr?.concealed != true || revealed
    val clock=LocalTextMotion.current?.clock
    val animate=LocalAppearance.current.messageEffects && !LocalMotion.current.reduced
    fun resultAlpha(full:Boolean)=if(!full && animate && value.motion!=null && (clock?.elapsed ?: 2000f)<RandomizerMotionMillis)0f else 1f
    val label = when (value.kind) {
        "calculation" -> "Calculation"; "conversion" -> "Conversion"; "math" -> "Formula"; "qr" -> when (qr?.kind) {
            "wifi" -> "Wi-Fi QR code"; "contact" -> "Contact QR code"; else -> "QR code"
        }
        "dice" -> "Dice"; "pick" -> if(value.motion?.kind=="coin")"Coin flip" else "Choice"; "random" -> "Random number"; "swatch" -> "Color"
        "keys" -> "Keyboard shortcut"; "rating" -> "Rating"; "progress" -> "Progress"; "quote" -> "Quote"; else -> "Details"
    }
    @Composable fun body(full: Boolean) {
        if (qr != null) {
            value.rich?.let { RichMessageText(it) }
            if (visible) QrImage(qr, Modifier.widthIn(max = if (full) 480.dp else 232.dp).fillMaxWidth().aspectRatio(1f))
            else SigilTextButton({ revealed = true }) { Glyph("visibility", 20); Text("Reveal QR code") }
            if (full && visible) {
                when (qr.kind) {
                    "wifi" -> {
                        Text("Scanning shares access to this network.", style = MaterialTheme.typography.bodyMedium)
                        SigilTextButton({ password = !password }) { Text(if (password) "Hide password" else "Show password") }
                        if (password) qr.password?.let { RichMessageText(it) }
                    }
                    "contact" -> Text("This code contains a contact address and key. It does not approve a device.", style = MaterialTheme.typography.bodyMedium)
                    else -> if (qr.kind == "url") SigilTextButton({ uri.openUri(qr.payload) }) { Text("Open link") }
                }
                SigilTextButton({ if (qr.kind == "wifi" && sensitiveCopy != null) sensitiveCopy(qr.payload) else clipboard.setText(AnnotatedString(qr.payload)) }) { Text(if (qr.kind == "wifi") "Copy Wi-Fi details" else "Copy QR contents") }
            }
        } else if (value.kind == "math") {
            val render = LocalMathContent.current
            if (render != null && value.mathml != null) render(value.mathml, value.display, Modifier.fillMaxWidth().height(if (full) 300.dp else 140.dp))
            else SelectionContainer { Text(value.display, fontFamily = LocalCodeFont.current) }
        } else {
            value.motion?.let {RandomizerStage(it,full,value.rich)}
            if(value.kind=="dice" && value.details.size>6)Text("Showing 6 of ${value.details.size} dice",style=MaterialTheme.typography.bodySmall)
            if (value.kind == "swatch") {
                val rgba = value.rgba ?: 0L
                val color = Color((rgba shr 24 and 255).toInt(), (rgba shr 16 and 255).toInt(), (rgba shr 8 and 255).toInt(), (rgba and 255).toInt())
                Canvas(Modifier.fillMaxWidth().height(if (full) 180.dp else 64.dp).semantics { contentDescription = "Color sample ${value.display}" }) {
                    val side = 12.dp.toPx()
                    for (y in 0..(size.height / side).toInt()) for (x in 0..(size.width / side).toInt())
                        drawRect(if ((x + y) % 2 == 0) Color.White else Color.LightGray, androidx.compose.ui.geometry.Offset(x * side, y * side), androidx.compose.ui.geometry.Size(side, side))
                    drawRect(color)
                }
            }
            if(value.motion?.kind!="choice")value.rich?.let { RichMessageText(it,Modifier.graphicsLayer {alpha=resultAlpha(full)}) }
            if (value.display.isNotEmpty() && !(value.kind=="random" && value.motion!=null) && value.motion?.kind!="coin") Text(if (swapped) value.alternate else value.display, modifier=Modifier.graphicsLayer {alpha=resultAlpha(full)}, style = if (value.kind in listOf("calculation", "random", "rating", "progress")) MaterialTheme.typography.headlineSmall else MaterialTheme.typography.bodyLarge)
            if (value.alternate.isNotEmpty()) {
                Text(if (swapped) value.display else value.alternate, style = MaterialTheme.typography.bodyLarge)
                if (value.kind == "conversion") SigilTextButton({ swapped = !swapped }) { Glyph("swap_vert", 20); Text("Swap display") }
            }
            value.ratio?.let { ratio ->
                val ink = LocalContentColor.current
                Canvas(Modifier.fillMaxWidth().height(8.dp).semantics { progressBarRangeInfo = ProgressBarRangeInfo(ratio.coerceIn(0f, 1f), 0f..1f) }) {
                    drawRoundRect(ink.copy(alpha = .15f), cornerRadius = androidx.compose.ui.geometry.CornerRadius(size.height / 2))
                    drawRoundRect(ink, size = size.copy(width = size.width * ratio.coerceIn(0f, 1f)), cornerRadius = androidx.compose.ui.geometry.CornerRadius(size.height / 2))
                }
            }
            value.secondary?.let { RichMessageText(it, style = MaterialTheme.typography.bodyMedium) }
            val details = if (full || value.kind in listOf("keys", "quote")) value.details else emptyList()
            details.forEachIndexed { index, detail ->
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (value.selected == index) Glyph("check", 20, "Selected choice")
                    if (value.kind == "keys") Surface(shape = MaterialTheme.shapes.small, color = LocalContentColor.current.copy(alpha = .08f)) {
                        RichMessageText(detail, Modifier.padding(10.dp), MaterialTheme.typography.bodyMedium.copy(fontFamily = LocalCodeFont.current))
                    } else RichMessageText(detail)
                }
            }
        }
    }
    Column(Modifier.widthIn(min = 200.dp, max = 280.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text(label, style = MaterialTheme.typography.labelMedium)
        body(false)
        SigilTextButton({ expanded = true }) { Glyph("open_in_full", 18); Spacer(Modifier.width(8.dp)); Text("Open ${label.lowercase()}") }
    }
    if (expanded) Dialog({ expanded = false }, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface) {
                Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        SigilIconButton({ expanded = false }) { Glyph("close", 24, "Close ${label.lowercase()}") }
                        Text(label, Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
                        value.copy?.let { copy -> SigilIconButton({ clipboard.setText(AnnotatedString(copy)) }) { Glyph("content_copy", 24, "Copy ${label.lowercase()}") } }
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
