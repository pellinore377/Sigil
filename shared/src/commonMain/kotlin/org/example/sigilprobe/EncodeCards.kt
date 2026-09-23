package org.sigil

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.isSpecified
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlin.math.floor

/** The card's one primary action: filled with ink, labelled in the ground colour. */
@Composable
internal fun CardInkAction(label: String, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val ink = LocalContentColor.current
    val ground = LocalMessageSurface.current.takeIf { it.isSpecified } ?: MaterialTheme.colorScheme.surface
    SigilButton(onClick, modifier.heightIn(min = 48.dp), colors = ButtonDefaults.buttonColors(containerColor = ink, contentColor = ground)) {
        Text(label, style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

// ---- ASCII art ----

internal const val ArtLineHeight = 1.3f

/** Uniform scale that fits the widest line; the card never scrolls, the enlarged view shows it 1:1. */
internal fun artScale(natural: Float, available: Float) = if (natural <= available || natural <= 0f) 1f else available / natural

private fun artDescription(lines: Int) = "ASCII art, $lines ${if (lines == 1) "line" else "lines"}"

@Composable
internal fun AsciiArtCard(art: String) {
    val lines = remember(art) { art.trimEnd('\n', '\r').lines().map { it.trimEnd() } }
    val text = remember(lines) { lines.joinToString("\n") }
    val base = MaterialTheme.typography.bodyMedium.copy(fontFamily = LocalCodeFont.current)
    val measurer = rememberTextMeasurer()
    var enlarged by remember(art) { mutableStateOf(false) }
    BoxWithConstraints(Modifier.padding(vertical = 4.dp)) {
        val natural = remember(text, base) { measurer.measure(AnnotatedString(text), base, softWrap = false).size.width.toFloat() }
        val scale = artScale(natural, constraints.maxWidth.toFloat())
        val size = base.fontSize * scale
        val shrunk = scale < 1f
        Text(text, (if (shrunk) Modifier.clip(RoundedCornerShape(14.dp)).clickable(onClickLabel = "Enlarge", role = Role.Button) { enlarged = true } else Modifier)
            .clearAndSetSemantics { contentDescription = artDescription(lines.size) + if (shrunk) ", shrunk to fit" else "" },
            style = base.copy(fontSize = size, lineHeight = size * ArtLineHeight), softWrap = false, maxLines = Int.MAX_VALUE)
    }
    if (enlarged) ArtDetails(text, lines.size) { enlarged = false }
}

@Composable
private fun ArtDetails(text: String, lines: Int, dismiss: () -> Unit) {
    Dialog(dismiss, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface, LocalContentColor provides MaterialTheme.colorScheme.onSurface) {
                Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        SigilIconButton(dismiss) { Glyph("close", 24, "Close ASCII art") }
                        Text("ASCII art", Modifier.weight(1f).padding(start = 8.dp), style = MaterialTheme.typography.titleLarge)
                    }
                    val code = MaterialTheme.typography.bodyMedium.copy(fontFamily = LocalCodeFont.current)
                    Box(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()).horizontalScroll(rememberScrollState())) {
                        Text(text, Modifier.semantics { contentDescription = artDescription(lines) }, style = code.copy(lineHeight = code.fontSize * ArtLineHeight), softWrap = false)
                    }
                }
            }
        }
    }
}

// ---- QR code ----

// sigil:contact:<address>:<identity>; the address itself contains a colon.
internal fun String.contactQrAddress() = removePrefix("sigil:contact:").substringBeforeLast(':')

private val QrTileShape = RoundedCornerShape(14.dp)

internal fun qrDescription(qr: QrContent, label: String?) = when (qr.kind) {
    "url" -> "QR code for ${qr.payload}"
    "wifi" -> "Wi-Fi QR code" + (label?.let { " for $it" } ?: "")
    "contact" -> "Contact QR code" + (label?.let { " for $it" } ?: "")
    else -> "QR code"
}

@Composable
internal fun QrCard(value: UtilityContent, mine: Boolean = false, open: (() -> Unit)? = null) {
    val qr = value.qr ?: return
    var revealed by remember(value) { mutableStateOf(false) }
    var enlarged by remember(value) { mutableStateOf(false) }
    val visible = !qr.concealed || revealed
    val side = qrSide(qr, 216.dp)
    Column(Modifier.padding(vertical = 4.dp).width(maxOf(side, MessageCardMinWidth)), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (visible) QrTile(qr, side, value.rich?.text, Modifier.align(Alignment.CenterHorizontally)) { enlarged = true }
        else SigilTextButton({ revealed = true }) { Glyph("visibility", 20); Spacer(Modifier.width(12.dp)); Text("Reveal QR code") }
        QrSummary(value, qr, visible)
        QrAction(qr, visible, mine, open, Modifier.fillMaxWidth())
    }
    if (enlarged) QrDetails(value, mine, open) { enlarged = false }
}

/** What scanning yields, so nobody scans blind: the payload, then meta that names anything withheld. */
@Composable
private fun QrSummary(value: UtilityContent, qr: QrContent, visible: Boolean) {
    val ink = LocalContentColor.current
    val quiet = ink.copy(alpha = .68f)
    val type = MaterialTheme.typography
    val code = type.labelMedium.copy(fontFamily = LocalCodeFont.current)
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        when (qr.kind) {
            "url" -> Text(qr.payload, style = code, maxLines = 3, overflow = TextOverflow.Ellipsis)
            "wifi" -> {
                value.rich?.let { RichMessageText(it, style = type.bodyMedium) }
                val open = qr.password?.text.isNullOrEmpty()
                Text(if (open) "Wi-Fi network · no password" else "Wi-Fi network · password hidden", style = type.labelMedium, color = quiet)
            }
            "contact" -> {
                value.rich?.let { Text(it.text, style = code, maxLines = 2, overflow = TextOverflow.Ellipsis) }
                Text("Sigil contact", style = type.labelMedium, color = quiet)
            }
            else -> if (visible) Text(qr.payload, style = type.bodyMedium, maxLines = 4, overflow = TextOverflow.Ellipsis)
                else Text("Hidden text", style = type.labelMedium, color = quiet)
        }
    }
}

@Composable
private fun QrAction(qr: QrContent, visible: Boolean, mine: Boolean, open: (() -> Unit)?, modifier: Modifier) {
    if (!visible) return
    val uri = LocalUriHandler.current
    val clipboard = LocalClipboardManager.current
    val sensitive = LocalSensitiveCopy.current
    when (qr.kind) {
        "url" -> CardInkAction("Open link", { uri.openUri(qr.payload) }, modifier)
        "wifi" -> if (!mine) qr.password?.text?.takeIf { it.isNotEmpty() }?.let { secret ->
            CardInkAction("Copy password", { sensitive?.invoke(secret) ?: clipboard.setText(AnnotatedString(secret)) }, modifier)
        }
        "contact" -> if (open != null) CardInkAction("Message", open, modifier.semantics { contentDescription = "Message ${qr.payload.contactQrAddress()}" })
    }
}

@Composable
private fun QrTile(qr: QrContent, side: Dp, label: String?, modifier: Modifier = Modifier, enlarge: (() -> Unit)?) {
    val description = qrDescription(qr, label)
    Canvas(modifier.size(side).clip(QrTileShape).background(Color.White)
        .then(if (enlarge != null) Modifier.clickable(onClickLabel = "Enlarge", role = Role.Button, onClick = enlarge) else Modifier)
        .semantics { contentDescription = description }) {
        if (qr.width !in 1..185) return@Canvas
        val module = floor(minOf(size.width, size.height) / qr.width)
        if (module < 1 || qr.cells.length != qr.width * qr.width) return@Canvas
        val origin = Offset(floor((size.width - module * qr.width) / 2), floor((size.height - module * qr.width) / 2))
        qr.cells.forEachIndexed { i, cell -> if (cell == '1') drawRect(Color.Black, origin + Offset(i % qr.width * module, i / qr.width * module), Size(module, module)) }
    }
}

// Snapped to whole modules: a fractional remainder would pad the tile with unscannable white.
@Composable
private fun qrSide(qr: QrContent, cap: Dp): Dp {
    val density = LocalDensity.current
    return remember(qr.width, cap, density.density) {
        if (qr.width !in 1..185) cap else with(density) { (floor(cap.toPx() / qr.width).coerceAtLeast(1f) * qr.width).toDp() }
    }
}

@Composable
internal fun QrDetails(value: UtilityContent, mine: Boolean = false, open: (() -> Unit)? = null, dismiss: () -> Unit) {
    val qr = value.qr ?: return
    var revealed by remember(value) { mutableStateOf(false) }
    var password by remember(value) { mutableStateOf(false) }
    val visible = !qr.concealed || revealed
    Dialog(dismiss, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface, LocalContentColor provides MaterialTheme.colorScheme.onSurface) {
                Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        SigilIconButton(dismiss) { Glyph("close", 24, "Close QR code") }
                        Text(utilityLabel(value), Modifier.weight(1f).padding(start = 8.dp), style = MaterialTheme.typography.titleLarge)
                    }
                    BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
                        val cap = minOf(maxWidth, 480.dp)
                        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(16.dp)) {
                            val side = qrSide(qr, cap)
                            if (visible) QrTile(qr, side, value.rich?.text, enlarge = null)
                            else SigilTextButton({ revealed = true }) { Glyph("visibility", 20); Spacer(Modifier.width(12.dp)); Text("Reveal QR code") }
                            Column(Modifier.widthIn(max = side.coerceAtLeast(MessageCardMinWidth)).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                                QrSummary(value, qr, visible)
                                QrAction(qr, visible, mine, open?.let { { dismiss(); it() } }, Modifier.fillMaxWidth())
                                if (visible) when (qr.kind) {
                                    "wifi" -> if (!qr.password?.text.isNullOrEmpty()) {
                                        SigilTextButton({ password = !password }) { Text(if (password) "Hide password" else "Show password") }
                                        if (password) qr.password?.let { RichMessageText(it, style = MaterialTheme.typography.bodyMedium.copy(fontFamily = LocalCodeFont.current)) }
                                    }
                                    "contact" -> Text("Scanning shares this address and key. It does not approve a device.", style = MaterialTheme.typography.bodyMedium)
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
