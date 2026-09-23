package org.sigil

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.isSpecified
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.layout.AlignmentLine
import androidx.compose.ui.layout.FirstBaseline
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.ceil
import kotlin.math.roundToInt

// ---- Pull quote ----

// The mark's top meets the first line's cap line; its ink ends this far below that, in ems.
private const val QuoteCapHeight = .66f
private const val QuoteMarkDepth = .42f
internal const val QuoteLines = 8

internal fun quoteAttribution(author: String?, source: String?) = listOfNotNull(author, source).joinToString(", ").takeIf { it.isNotEmpty() }?.let { "— $it" }

@Composable
internal fun PullQuoteCard(value: UtilityContent) {
    val ink = LocalContentColor.current
    val quiet = ink.copy(alpha = .68f)
    val author = value.secondary?.text?.trim()?.takeIf { it.isNotEmpty() }
    val source = value.details.firstOrNull()?.text?.trim()?.takeIf { it.isNotEmpty() }
    val type = MaterialTheme.typography
    val body = type.titleMedium.copy(fontStyle = FontStyle.Italic)
    val mark = type.displayLarge.let { it.copy(fontSize = it.fontSize * 2f, lineHeight = it.fontSize * 2f) }
    var expanded by remember(value) { mutableStateOf(false) }
    Layout({
        Text("“", Modifier.clearAndSetSemantics {}, style = mark, color = ink.copy(alpha = .56f), maxLines = 1)
        Column(Modifier.animateContentSize(LocalMotion.current.tween(MotionMillis)), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            value.rich?.let { rich ->
                BoxWithConstraints {
                    val measurer = rememberTextMeasurer()
                    val long = remember(rich.text, constraints.maxWidth, body) { measurer.measure(rich.text, body, constraints = Constraints(maxWidth = constraints.maxWidth)).lineCount > QuoteLines }
                    Column {
                        RichMessageText(rich, style = body, maxLines = if (long && !expanded) QuoteLines else Int.MAX_VALUE)
                        if (long) Text(if (expanded) "Show less" else "Show more", Modifier.heightIn(min = 48.dp).clickable(role = Role.Button) { expanded = !expanded }
                            .wrapContentHeight(Alignment.CenterVertically), style = type.bodyMedium, color = ink)
                    }
                }
            }
            quoteAttribution(author, source)?.let { spoken ->
                // A drawn em rule: the face's own dash reads as a hyphen at this size.
                var baseline by remember { mutableFloatStateOf(0f) }
                val em = with(LocalDensity.current) { type.labelMedium.fontSize.toDp() }
                Text(buildAnnotatedString {
                    author?.let { append(it) }
                    source?.let { withStyle(SpanStyle(color = quiet, fontStyle = FontStyle.Italic)) { append(if (author != null) ", $it" else it) } }
                }, Modifier.clearAndSetSemantics { contentDescription = spoken }.padding(start = em * 1.45f).drawBehind {
                    val y = baseline - type.labelMedium.fontSize.toPx() * .3f
                    drawRect(quiet, Offset(-(em * 1.45f).toPx(), y), Size(em.toPx(), 1.dp.toPx()))
                }, style = type.labelMedium, onTextLayout = { baseline = it.firstBaseline })
            }
        }
    }, Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).padding(vertical = 4.dp).semantics(mergeDescendants = true) { contentDescription = "Quote" }) { measurables, constraints ->
        val gutter = 44.dp.roundToPx()
        val markPlaced = measurables[0].measure(Constraints())
        val column = measurables[1].measure(Constraints(maxWidth = (constraints.maxWidth - gutter).coerceAtLeast(0), maxHeight = constraints.maxHeight))
        val first = column[FirstBaseline].takeIf { it != AlignmentLine.Unspecified }?.toFloat() ?: (body.lineHeight.toPx() * .76f)
        val capTop = first - body.fontSize.toPx() * QuoteCapHeight
        val markY = (capTop + mark.fontSize.toPx() * QuoteCapHeight - markPlaced[FirstBaseline]).roundToInt()
        val width = (gutter + column.width).coerceIn(constraints.minWidth, constraints.maxWidth)
        val height = maxOf(column.height, ceil(capTop + mark.fontSize.toPx() * QuoteMarkDepth).toInt()).coerceIn(constraints.minHeight, constraints.maxHeight)
        layout(width, height) { markPlaced.place((-3).dp.roundToPx(), markY); column.place(gutter, 0) }
    }
}

// ---- Keyboard shortcut ----

// One cap: the Material Symbols glyph (bundled fonts lack ⇧ ⌘ ↵), the printed word, and how it is spoken.
internal data class KeyFace(val glyph: String?, val word: String, val spoken: String)

// A combination with Cmd or Option is a Mac shortcut, where Control carries its ⌃ legend.
internal fun macCombo(keys: List<String>) = keys.any { it.trim().lowercase() in setOf("cmd", "command", "⌘", "opt", "option", "⌥") }

internal fun keyFace(raw: String, mac: Boolean = false): KeyFace {
    val key = raw.trim()
    return when (key.lowercase()) {
        "cmd", "command", "⌘" -> KeyFace("keyboard_command_key", "Cmd", "Command")
        "opt", "option", "⌥" -> KeyFace("keyboard_option_key", "Option", "Option")
        "ctrl", "control", "⌃" -> if (mac || key == "⌃") KeyFace("keyboard_control_key", "Control", "Control") else KeyFace(null, "Ctrl", "Control")
        "shift", "⇧" -> KeyFace("shift", "Shift", "Shift")
        "enter", "↵", "⏎" -> KeyFace("keyboard_return", "Enter", "Enter")
        "return" -> KeyFace("keyboard_return", "Return", "Return")
        "tab", "⇥" -> KeyFace("keyboard_tab", "Tab", "Tab")
        "backspace", "⌫" -> KeyFace("backspace", "Backspace", "Backspace")
        "esc", "escape", "⎋" -> KeyFace(null, "Esc", "Escape")
        "del", "delete", "⌦" -> KeyFace(null, "Delete", "Delete")
        "alt" -> KeyFace(null, "Alt", "Alt")
        "fn" -> KeyFace(null, "fn", "Function")
        "space", "spacebar" -> KeyFace(null, "Space", "Space")
        "up", "arrowup", "↑" -> KeyFace("arrow_upward", "", "Up arrow")
        "down", "arrowdown", "↓" -> KeyFace("arrow_downward", "", "Down arrow")
        "left", "arrowleft", "←" -> KeyFace("arrow_back", "", "Left arrow")
        "right", "arrowright", "→" -> KeyFace("arrow_forward", "", "Right arrow")
        "pgup", "pageup" -> KeyFace(null, "PgUp", "Page up")
        "pgdn", "pagedown" -> KeyFace(null, "PgDn", "Page down")
        else -> if (key.length == 1) KeyFace(null, key.uppercase(), key.uppercase()) else KeyFace(null, key, key)
    }
}

internal fun keysNamed(keys: List<String>) = macCombo(keys).let { mac -> keys.joinToString(" plus ") { keyFace(it, mac).spoken } }

internal fun keysSpoken(keys: List<String>) = "Keyboard shortcut. " + keysNamed(keys)

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun KeysCard(value: UtilityContent) {
    val keys = value.details.map { it.text }
    FlowRow(Modifier.widthIn(max = MessageCardMaxWidth).padding(vertical = 4.dp).clearAndSetSemantics { contentDescription = keysSpoken(keys) },
        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) { KeyRun(value, false) }
}

// The joiner travels with the key after it, so a wrap never strands a "+" at a line end.
@Composable
private fun KeyRun(value: UtilityContent, compact: Boolean) {
    val quiet = LocalContentColor.current.copy(alpha = .68f)
    val mac = macCombo(value.details.map { it.text })
    value.details.forEachIndexed { index, detail ->
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(if (compact) 4.dp else 8.dp)) {
            if (index > 0) Text("+", style = MaterialTheme.typography.bodyMedium.copy(fontFamily = LocalCodeFont.current), color = quiet)
            Keycap(keyFace(detail.text, mac), detail, compact)
        }
    }
}

// A shortcut inside a sentence stays in the line, as it is used in chat.
internal fun ChatMessage.inlineKeysLine(): Boolean {
    fun MessagePart.plain() = rich?.text ?: text
    return parts.any { it.utility?.kind == "keys" } && parts.any { it.utility == null && it.kind == "text" && it.plain().isNotBlank() } &&
        parts.all { p -> p.utility?.kind == "keys" || (p.utility == null && p.kind == "text" && '\n' !in p.plain().trim() && p.rich?.blocks.isNullOrEmpty() && p.rich?.motion.isNullOrEmpty()) }
}

// A word or a shortcut; pieces in one group sit together with no space between.
internal class KeyFlowPiece(val word: RichText? = null, val plain: String? = null, val keys: UtilityContent? = null)

private val closing = ".,;:!?)]}’”…"
private val opening = "([{‘“"

internal fun keyFlow(parts: List<MessagePart>): List<List<KeyFlowPiece>> {
    val groups = mutableListOf<MutableList<KeyFlowPiece>>()
    var glue = false
    parts.forEach { part ->
        val keys = part.utility?.takeIf { it.kind == "keys" }
        if (keys != null) {
            if (glue && groups.isNotEmpty()) groups.last() += KeyFlowPiece(keys = keys) else groups += mutableListOf(KeyFlowPiece(keys = keys))
            glue = false
            return@forEach
        }
        val source = part.rich?.text ?: part.text
        Regex("\\S+").findAll(source).forEachIndexed { index, match ->
            val piece = if (part.rich != null) KeyFlowPiece(word = richSlice(part.rich, match.range.first, match.range.last + 1)) else KeyFlowPiece(plain = match.value)
            val afterKeys = index == 0 && groups.lastOrNull()?.lastOrNull()?.keys != null && match.value.first() in closing && match.range.first == 0
            if (afterKeys) groups.last() += piece else groups += mutableListOf(piece)
        }
        glue = source.isNotEmpty() && !source.last().isWhitespace() && source.trimEnd().lastOrNull()?.let { it in opening } == true
    }
    return groups
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun InlineKeysMessage(message: ChatMessage, analyze: (String) -> String) {
    val style = MaterialTheme.typography.bodyLarge
    val measurer = rememberTextMeasurer()
    val space = with(LocalDensity.current) { remember(style) { (measurer.measure("a a", style).size.width - measurer.measure("aa", style).size.width).toDp() } }
    val groups = remember(message.parts) { keyFlow(message.parts) }
    FlowRow(Modifier.widthIn(max = MessageCardMaxWidth).semantics(mergeDescendants = true) {}, horizontalArrangement = Arrangement.spacedBy(space), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        groups.forEach { group ->
            Row(Modifier.alignByBaseline()) {
                group.forEach { piece ->
                    val keys = piece.keys
                    when {
                        keys != null -> Row(Modifier.alignByBaseline().clearAndSetSemantics { contentDescription = keysNamed(keys.details.map { it.text }) },
                            verticalAlignment = Alignment.CenterVertically) { KeyRun(keys, true) }
                        piece.word != null -> Box(Modifier.alignByBaseline()) { RichMessageText(piece.word) }
                        else -> Box(Modifier.alignByBaseline()) { MessageText(piece.plain.orEmpty(), analyze) }
                    }
                }
            }
        }
    }
}

// Lit from above, derived from the bubble ground: a paler face on a body that darkens to a deep front edge.
private data class KeyShade(val faceTop: Color, val faceBottom: Color, val bodyTop: Color, val bodyBottom: Color, val edge: Color, val rim: Color,
    val faceRim: Color, val highlight: Color, val drop: Color)

private fun keyShade(ground: Color): KeyShade = if (ground.luminance() > .4f) lerp(ground, Color.White, .9f).let { top -> KeyShade(
    top, lerp(ground, Color.White, .45f), lerp(ground, Color.Black, .06f), lerp(ground, Color.Black, .2f),
    lerp(ground, Color.Black, .3f), lerp(ground, Color.Black, .16f), lerp(top, Color.White, .5f), Color.White, Color.Black.copy(alpha = .1f),
) } else lerp(ground, Color.White, .24f).let { top -> KeyShade(
    top, lerp(ground, Color.White, .12f), lerp(ground, Color.White, .14f), lerp(ground, Color.Black, .12f),
    lerp(ground, Color.Black, .5f), lerp(ground, Color.Black, .35f), lerp(top, Color.White, .12f), Color.White.copy(alpha = .08f), Color.Black.copy(alpha = .27f),
) }

@Composable
private fun Keycap(face: KeyFace, detail: RichText, compact: Boolean = false) {
    val ink = LocalContentColor.current
    val shade = keyShade(LocalMessageSurface.current.takeIf { it.isSpecified } ?: MaterialTheme.colorScheme.surface)
    val style = MaterialTheme.typography.labelMedium.copy(fontFamily = LocalCodeFont.current, fontWeight = FontWeight.Medium)
    val space = face.word == "Space"
    val radius = if (compact) 8.dp else 10.dp
    val depth = if (compact) 2.dp else 3.dp
    val body = RoundedCornerShape(radius)
    Box(Modifier.padding(bottom = depth).drawBehind {
        // The front edge: the body's own shape dropped, so the key stands on the bubble.
        drawRoundRect(shade.edge, topLeft = Offset(0f, depth.toPx()), size = size, cornerRadius = CornerRadius(radius.toPx()))
    }.clip(body).background(Brush.verticalGradient(listOf(shade.bodyTop, shade.bodyBottom))).border(1.dp, shade.rim, body)
        .widthIn(min = if (space) 96.dp else if (compact) 30.dp else 42.dp).heightIn(min = if (compact) 30.dp else 47.dp)
        .padding(start = 4.dp, end = 4.dp, top = if (compact) 2.dp else 3.dp, bottom = if (compact) 5.dp else 8.dp)) {
        Row(Modifier.drawBehind {
            // Face: a lighter rim, a 1dp top highlight inside it, and a 1dp drop onto the body.
            val r = CornerRadius(7.dp.toPx()); val px = 1.dp.toPx()
            drawRoundRect(shade.drop, topLeft = Offset(0f, px), size = size, cornerRadius = r)
            drawRoundRect(shade.faceRim, size = size, cornerRadius = r)
            drawRoundRect(shade.highlight, topLeft = Offset(px, px), size = Size(size.width - 2 * px, size.height - 2 * px), cornerRadius = r)
            drawRoundRect(Brush.linearGradient(listOf(shade.faceTop, shade.faceBottom), Offset.Zero, Offset(size.width, size.height)), topLeft = Offset(px, 2 * px),
                size = Size(size.width - 2 * px, size.height - 3 * px), cornerRadius = r)
        }.widthIn(min = if (space) 88.dp else if (compact) 22.dp else 31.dp).heightIn(min = if (compact) 23.dp else 35.dp).padding(horizontal = if (compact) 6.dp else 10.dp).align(Alignment.Center),
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp, Alignment.CenterHorizontally)) {
            CompositionLocalProvider(LocalContentColor provides ink) {
                face.glyph?.let { Glyph(it, if (compact) 14 else 16) }
                if (face.word.isNotEmpty()) {
                    if (face.word == detail.text.trim() && detail.spans.isNotEmpty()) RichMessageText(detail, style = style)
                    else Text(face.word, style = style, maxLines = 1)
                }
            }
        }
    }
}


// ---- Color swatch ----

// Opaque #RRGGBB from the core's 0xRRGGBBAA; alpha is reported in words, never shown as see-through.
internal fun swatchHex(rgba: Long): String = "#" + ((rgba ushr 8) and 0xFFFFFF).toString(16).padStart(6, '0').uppercase()

internal fun swatchOpacity(rgba: Long): Int? = (rgba and 0xFF).toInt().takeIf { it < 255 }?.let { (it * 100 / 255.0).roundToInt() }

internal fun swatchColor(rgba: Long) = Color(((rgba ushr 24) and 255).toInt(), ((rgba ushr 16) and 255).toInt(), ((rgba ushr 8) and 255).toInt())

// A rough name so the colour is not carried by hue alone.
internal fun swatchName(rgba: Long): String {
    val r = ((rgba ushr 24) and 255) / 255f; val g = ((rgba ushr 16) and 255) / 255f; val b = ((rgba ushr 8) and 255) / 255f
    val max = maxOf(r, g, b); val min = minOf(r, g, b); val l = (max + min) / 2; val d = max - min
    if (d < .08f) return when { l > .92f -> "white"; l < .1f -> "black"; l > .6f -> "light grey"; l < .3f -> "dark grey"; else -> "grey" }
    val h = (when (max) { r -> ((g - b) / d).let { if (it < 0) it + 6 else it }; g -> (b - r) / d + 2; else -> (r - g) / d + 4 } * 60f)
    val hue = when { h < 10 || h >= 345 -> "red"; h < 40 -> if (l < .4f) "brown" else "orange"; h < 65 -> "yellow"; h < 160 -> "green"; h < 195 -> "teal"
        h < 255 -> "blue"; h < 290 -> "purple"; else -> "pink" }
    return when { l > .75f -> "light $hue"; l < .28f -> "dark $hue"; else -> hue }
}

internal fun swatchSpoken(rgba: Long) = "Color ${swatchHex(rgba)}, ${swatchName(rgba)}" + (swatchOpacity(rgba)?.let { ", $it% opacity" } ?: "")

@Composable
internal fun SwatchCard(value: UtilityContent) = SwatchPalette(listOf(value))

// One tile alone, or a run of swatches as a wrapping palette of smaller tiles.
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun SwatchPalette(values: List<UtilityContent>) {
    val single = values.size == 1
    // Four fall into two even rows rather than three and a stray.
    FlowRow(Modifier.widthIn(max = MessageCardMaxWidth).padding(vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp), maxItemsInEachRow = if (values.size == 4) 2 else 3) {
        values.forEach { SwatchTile(it.rgba ?: 0L, if (single) MessageCardMinWidth else 96.dp, if (single) 96.dp else 64.dp) }
    }
}

@Composable
private fun SwatchTile(rgba: Long, width: Dp, height: Dp) {
    val ink = LocalContentColor.current
    // No border, even where the colour meets the bubble: the value under it names the colour.
    Column(Modifier.width(width).clearAndSetSemantics { contentDescription = swatchSpoken(rgba) }, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(Modifier.fillMaxWidth().height(height).clip(RoundedCornerShape(14.dp)).background(swatchColor(rgba)))
        Column(verticalArrangement = Arrangement.spacedBy(0.dp)) {
            Text(swatchHex(rgba), style = MaterialTheme.typography.bodyMedium.copy(fontFamily = LocalCodeFont.current, fontFeatureSettings = "tnum, lnum"), maxLines = 1, overflow = TextOverflow.Ellipsis)
            swatchOpacity(rgba)?.let { Text("$it% opacity", style = MaterialTheme.typography.labelMedium, color = ink.copy(alpha = .68f), maxLines = 1) }
        }
    }
}

private fun MessagePart.isSwatch() = utility?.kind == "swatch"
private fun MessagePart.isGap() = kind == "text" && (rich?.text ?: text).isBlank()

// Swatches separated only by blank text read as one palette: the run's first index draws it, the rest draw nothing.
internal fun swatchRun(parts: List<MessagePart>, index: Int): List<UtilityContent>? {
    if (!parts[index].isSwatch() && !parts[index].isGap()) return null
    var start = index
    while (start > 0 && (parts[start - 1].isSwatch() || parts[start - 1].isGap())) start--
    var end = index
    while (end < parts.lastIndex && (parts[end + 1].isSwatch() || parts[end + 1].isGap())) end++
    val run = parts.subList(start, end + 1).mapNotNull { if (it.isSwatch()) it.utility else null }
    if (run.size < 2) return null
    val first = (start..end).first { parts[it].isSwatch() }
    return if (index == first) run else emptyList()
}

// Menu Copy for a message of share cards: the values people paste, not the SigilText source.
internal fun shareCopy(message: ChatMessage): String? {
    if (message.inlineKeysLine()) return keyFlow(message.parts).joinToString(" ") { group ->
        group.joinToString("") { it.keys?.details?.joinToString("+") { key -> key.text.trim() } ?: it.word?.quotePlain() ?: it.plain.orEmpty() }
    }
    val cards = message.parts.filterNot { it.utility == null && it.kind == "text" && (it.rich?.text ?: it.text).isBlank() }
    if (cards.isEmpty() || cards.any { it.utility?.kind !in setOf("swatch", "keys", "quote") }) return null
    return cards.joinToString("\n") { part ->
        val value = part.utility!!
        when (value.kind) {
            "swatch" -> value.rgba?.let { rgba -> swatchHex(rgba) + (swatchOpacity(rgba)?.let { (rgba and 0xFF).toString(16).padStart(2, '0').uppercase() } ?: "") } ?: value.display
            "keys" -> value.details.joinToString("+") { it.text.trim() }
            else -> listOfNotNull(value.rich?.quotePlain()?.trim(), quoteAttribution(value.secondary?.text?.trim()?.takeIf { it.isNotEmpty() },
                value.details.firstOrNull()?.text?.trim()?.takeIf { it.isNotEmpty() })).joinToString("\n")
        }
    }
}
