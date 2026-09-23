package org.sigil

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Fill
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

val QuotePreviewSize = 48.dp

// What a quoted attachment is called beneath the sender's name.
fun attachmentLabel(file: AttachmentDetails): String = when {
    file.name == "Voice message.aac" -> "Voice message"
    file.mediaType == "image/gif" -> "GIF"
    file.mediaType.startsWith("image/") -> "Photo"
    file.mediaType.startsWith("video/") -> "Video"
    file.mediaType.startsWith("audio/") -> "Audio"
    else -> attachmentKind(file.name, file.mediaType).name
}

private fun attachmentGlyph(file: AttachmentDetails): String = when {
    file.name == "Voice message.aac" -> "mic"
    file.mediaType.startsWith("image/") -> "image"
    file.mediaType.startsWith("video/") -> "videocam"
    else -> attachmentKind(file.name, file.mediaType).glyph
}

// A quoted card: the Create panel's glyph and kind, then what tells it apart from its siblings.
data class CardQuote(val kind: String, val glyph: String, val detail: String = "", val chart: ChartContent? = null, val math: UtilityContent? = null, val rgba: Long? = null) {
    val line get() = if (detail.isBlank()) kind else "$kind · $detail"
}

// Plain quote text on one line; a spoiler or redaction stays covered.
internal fun RichText.quotePlain(): String = buildString {
    var at = 0
    spans.sortedBy { it.start }.forEach { span ->
        if (span.start < at || span.end > text.length) return@forEach
        if (span.reveal.isNotEmpty() || span.redaction > 0) { appendRange(text, at, span.start); append("•••"); at = span.end }
    }
    appendRange(text, at, text.length)
}.replace(Regex("\\s+"), " ").trim()

private fun MessagePart.title() = rich?.quotePlain()?.takeIf { it.isNotBlank() } ?: text.replace(Regex("\\s+"), " ").trim()
private fun details(vararg values: String?) = values.filterNot { it.isNullOrBlank() }.joinToString(" · ")

// What a text-only quote reads; concealed spans stay covered.
fun quoteText(parts: List<MessagePart>, fallback: String?): String? =
    if (parts.isNotEmpty() && parts.all { it.kind == "text" } && parts.any { p -> p.rich?.spans.orEmpty().any { it.reveal.isNotEmpty() || it.redaction > 0 } }) parts.joinToString(" ") { it.title() } else fallback

private fun diceLine(utility: UtilityContent): String {
    val faces = utility.details.mapNotNull { it.text.substringAfter(" · ", "").takeIf(String::isNotBlank) }
    val total = utility.motion?.result?.takeIf { it.isNotBlank() } ?: faces.sumOf { it.toLongOrNull() ?: 0 }.toString()
    return when {
        faces.size <= 1 -> total
        faces.size <= 6 -> faces.joinToString(" + ") + " = " + total
        else -> faces.take(6).joinToString(" + ") + " + … = " + total
    }
}

fun cardQuote(parts: List<MessagePart>): CardQuote? {
    val part = parts.firstOrNull { it.kind != "text" } ?: return null
    val utility = part.utility
    val service = part.service
    val chart = part.chart
    val diagram = part.diagram
    val table = part.table
    return when {
        part.kind == "poll" -> CardQuote("Poll", "ballot", part.title())
        part.kind in listOf("checklist", "task", "recurring") -> CardQuote(when (part.kind) { "task" -> "Task"; "recurring" -> "Recurring checklist"; else -> "Checklist" },
            when (part.kind) { "task" -> "assignment"; "recurring" -> "event_repeat"; else -> "checklist" },
            details(part.title().ifBlank { part.items.firstOrNull()?.let { it.rich?.quotePlain() ?: it.text } }, part.items.takeIf { it.isNotEmpty() }?.let { "${it.count(CardItem::checked)} of ${it.size} done" }))
        part.kind == "note" -> CardQuote("Note", "description", part.title())
        part.kind == "reminder" -> CardQuote("Reminder", "notifications_active", details(part.title(), part.date))
        part.kind == "countdown" -> CardQuote("Countdown", "hourglass_bottom", details(part.title(), part.date))
        part.kind == "ago" -> CardQuote("Elapsed time", "history", details(part.title(), part.date))
        part.kind == "timer" -> CardQuote("Timer", "timer", details(part.title(), part.date))
        part.kind == "location" -> CardQuote("Location", "location_on", part.title().takeIf { it.isNotBlank() && it !in listOf("My location", "Dropped pin") }
            ?: when (part.locationMode) { "live" -> "Live location"; "once" -> "Shared location"; else -> "Dropped pin" })
        table != null -> CardQuote("Table", "table", details(table.columns.joinToString(", ") { it.quotePlain() }, "${table.rows.size} ${if (table.rows.size == 1) "row" else "rows"}"))
        part.recipe != null -> CardQuote("Recipe", "restaurant", details(part.recipe.title.quotePlain(), part.recipe.serves?.let { "serves $it" }))
        chart != null -> CardQuote("${chartKindName(chart.kind)} chart", "bar_chart", chart.title.quotePlain().ifBlank { chart.points.indices.take(3).joinToString(", ") { chart.pointName(it) } }, chart = chart)
        diagram != null -> CardQuote(diagramKindWord(diagram.kind), "account_tree", diagram.title.quotePlain().ifBlank { diagram.nodes.take(3).joinToString(" → ") { it.label.quotePlain() } })
        part.contact != null -> CardQuote("Contact", "person", part.contact.name.quotePlain().ifBlank { part.contact.address })
        service != null -> when (service.kind.lowercase()) {
            "translation" -> CardQuote("Translation", "translate", service.title.quotePlain())
            "definition" -> CardQuote("Definition", "dictionary", service.title.quotePlain())
            else -> CardQuote("Weather", "partly_cloudy_day", details(service.title.quotePlain(), service.current?.temperature?.firstOrNull()))
        }
        utility != null -> when (utility.kind) {
            "calculation" -> CardQuote("Calculation", "calculate", listOfNotNull(utility.rich?.let { r -> (if (r.spans.isEmpty()) arithPlain(r.text) else null) ?: r.quotePlain() }?.takeIf { it.isNotBlank() },
                readableNumber(utility.display).takeIf { it.isNotBlank() }).joinToString(" = "))
            "conversion" -> CardQuote("Conversion", "swap_horiz", listOf(utility.display, utility.alternate).filter { it.isNotBlank() }.joinToString(" = "))
            "math", "formula" -> CardQuote("Math", "functions", utility.display, math = utility)
            "qr" -> CardQuote("QR code", "qr_code", if (utility.qr?.concealed == true) "•••" else utility.rich?.quotePlain().orEmpty())
            "dice" -> CardQuote("Dice", "casino", diceLine(utility))
            "coin" -> CardQuote("Coin", "toll", utility.motion?.result.orEmpty())
            "pick" -> if (utility.motion?.kind == "coin") CardQuote("Coin", "toll", utility.motion.result.ifBlank { utility.motion.frames.getOrNull(utility.motion.selected).orEmpty() })
                else CardQuote(utility.display.takeIf { it.isNotBlank() && it != "Choice" }?.replaceFirstChar { it.uppercase() } ?: "Pick", "playing_cards",
                    utility.rich?.quotePlain()?.takeIf { it.isNotBlank() } ?: utility.motion?.result.orEmpty())
            "random" -> CardQuote("Random number", "numbers", details(utility.motion?.result?.takeIf { it.isNotBlank() } ?: utility.display, utility.alternate))
            "swatch" -> CardQuote("Color swatch", "palette", utility.display, rgba = utility.rgba)
            "keys" -> CardQuote("Keyboard shortcut", "keyboard", utility.details.joinToString(" + ") { it.quotePlain() })
            "rating" -> CardQuote("Rating", "star", utility.display)
            "progress" -> CardQuote("Progress", "data_usage", details(utility.display, utility.rich?.quotePlain()))
            "quote" -> CardQuote("Quote", "format_quote", details(utility.rich?.quotePlain()?.takeIf { it.isNotBlank() }?.let { "“$it”" }, utility.secondary?.quotePlain()))
            "art" -> CardQuote("ASCII art", "draw")
            else -> CardQuote("Card", "data_object")
        }
        else -> CardQuote("Card", "data_object", part.title())
    }
}

// A chart's shape at thumbnail size: one ink, the slices on a ramp as the card draws them.
@Composable
private fun ChartThumbnail(chart: ChartContent, modifier: Modifier) {
    val ink = LocalContentColor.current
    val points = chart.points.take(24)
    Canvas(modifier) {
        if (points.isEmpty()) return@Canvas
        val ramp = listOf(1f, .7f, .46f, .28f)
        when (chart.kind) {
            "pie", "donut" -> {
                var angle = -90f
                val stroke = if (chart.kind == "donut") Stroke(size.minDimension * .22f) else null
                val inset = (stroke?.width ?: 0f) / 2
                points.forEachIndexed { i, p ->
                    val sweep = p.share * 360f
                    drawArc(ink.copy(alpha = ramp[i % ramp.size]), angle, sweep, stroke == null, Offset(inset, inset), Size(size.width - 2 * inset, size.height - 2 * inset), style = stroke ?: Fill)
                    angle += sweep
                }
            }
            "bar" -> {
                val slot = (if (chart.horizontal) size.height else size.width) / points.size
                points.forEachIndexed { i, p ->
                    val lo = minOf(p.y, chart.zero); val hi = maxOf(p.y, chart.zero)
                    if (chart.horizontal) drawRect(ink.copy(alpha = .72f), Offset(size.width * lo, slot * (i + .2f)), Size(size.width * (hi - lo).coerceAtLeast(.02f), slot * .6f))
                    else drawRect(ink.copy(alpha = .72f), Offset(slot * (i + .2f), size.height * (1 - hi)), Size(slot * .6f, size.height * (hi - lo).coerceAtLeast(.02f)))
                }
            }
            "scatter" -> points.forEach { drawCircle(ink, 1.6.dp.toPx(), Offset(it.x * size.width, (1 - it.y) * size.height)) }
            else -> {
                val line = Path().apply { points.forEachIndexed { i, p -> val o = Offset(p.x * size.width, (1 - p.y) * size.height); if (i == 0) moveTo(o.x, o.y) else lineTo(o.x, o.y) } }
                if (chart.kind == "area") drawPath(Path().apply { addPath(line); lineTo(points.last().x * size.width, size.height * (1 - chart.zero)); lineTo(points.first().x * size.width, size.height * (1 - chart.zero)); close() }, ink.copy(alpha = .12f))
                drawPath(line, ink, style = Stroke(1.5.dp.toPx(), cap = StrokeCap.Round, join = StrokeJoin.Round))
            }
        }
    }
}

// The formula itself, typeset and scaled to the quote's line; the TeX only where no layout came.
@Composable
private fun QuoteFormula(value: UtilityContent) {
    val typeset = value.math
    if (typeset == null) { Text(value.display, style = MaterialTheme.typography.bodySmall, fontFamily = LocalCodeFont.current, maxLines = 1, overflow = TextOverflow.Ellipsis); return }
    BoxWithConstraints(Modifier.clipToBounds().clearAndSetSemantics { contentDescription = "Formula. ${value.display}" }) {
        val body = MaterialTheme.typography.bodyMedium.fontSize
        val density = LocalDensity.current
        val em = with(density) {
            val room = if (constraints.hasBoundedWidth) constraints.maxWidth / typeset.width.coerceAtLeast(.1f) else Float.MAX_VALUE
            minOf(body.toPx(), room, 36.dp.toPx() / (typeset.ascent + typeset.descent + .16f).coerceAtLeast(.1f)).coerceAtLeast(10.sp.toPx()).toSp()
        }
        // Past the smallest legible size it clips at the quote's edge rather than scrolling under a swipe.
        val natural = with(density) { (typeset.width * em.toPx()).toDp() + 1.dp }
        TypesetMath(typeset, em, Modifier.wrapContentWidth(Alignment.Start, unbounded = true).requiredWidth(natural))
    }
}

// A squircle glimpse of the quoted file: the picture or poster where one can be drawn, otherwise its glyph.
@Composable
fun QuotePreview(file: AttachmentDetails, source: ChatMessage?) {
    val ink = LocalContentColor.current
    Box(Modifier.size(QuotePreviewSize).clip(RoundedCornerShape(12.dp)).background(ink.copy(alpha = .1f)), contentAlignment = Alignment.Center) {
        val drawn = source != null && source.attachment != null && (file.mediaType.startsWith("image/") || file.mediaType.startsWith("video/")) && LocalAttachmentThumbnail.current(source, Modifier.matchParentSize())
        if (!drawn) Glyph(attachmentGlyph(file), 24, filled = file.name == "Voice message.aac")
    }
}

// The body shared by the bubble quote and the composer chip: name over text, or a preview beside name and type.
@Composable
fun QuoteBody(name: String?, text: String?, file: AttachmentDetails?, source: ChatMessage?, lines: Int, modifier: Modifier = Modifier, card: CardQuote? = null) {
    val title = MaterialTheme.typography.labelMedium.copy(fontWeight = FontWeight.SemiBold)
    if (file == null && card != null) Row(modifier, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        val ink = LocalContentColor.current
        val swatch = card.rgba?.let { Color((it ushr 8).toInt() or ((it and 0xff).toInt() shl 24)) }
        Box(Modifier.size(QuotePreviewSize).clip(RoundedCornerShape(12.dp)).background(swatch ?: ink.copy(alpha = .1f)), contentAlignment = Alignment.Center) {
            if (card.chart != null) ChartThumbnail(card.chart, Modifier.padding(10.dp).fillMaxSize()) else if (swatch == null) Glyph(card.glyph, 24)
        }
        Column {
            if (name != null) Text(name, style = title, maxLines = 1, overflow = TextOverflow.Ellipsis)
            val quiet = ink.copy(alpha = .68f)
            if (card.math != null) QuoteFormula(card.math)
            else Text(buildAnnotatedString { withStyle(SpanStyle(color = quiet)) { append(card.kind) }; if (card.detail.isNotBlank()) append(" · ${card.detail}") },
                style = MaterialTheme.typography.bodySmall, maxLines = if (lines > 1) 2 else 1, overflow = TextOverflow.Ellipsis)
        }
    } else if (file != null) Row(modifier, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        QuotePreview(file, source)
        Column {
            if (name != null) Text(name, style = title, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(attachmentLabel(file), style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    } else Column(modifier) {
        if (name != null) Text(name, style = title, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (text != null) Text(text, style = MaterialTheme.typography.bodySmall, maxLines = lines, overflow = TextOverflow.Ellipsis)
    }
}

// The quoted message set into a bubble: the timeline ground, so it reads darker in dark mode and lighter in light; its bottom corners meet the text.
@Composable
internal fun ReplyQuote(message: ChatMessage, topStart: androidx.compose.ui.unit.Dp = 14.dp, topEnd: androidx.compose.ui.unit.Dp = 14.dp) {
    val quoted = message.reply ?: return
    val scheme = MaterialTheme.colorScheme
    val name = message.replyAuthor?.let { LocalMediaSender.current(message.copy(author = it, mine = message.replyMine)) }
    // The quote names the file itself, so a thumbnail never depends on the quoted message being in the loaded list.
    val source = if (message.replyAuthor != null && message.replyMessage != null) LocalMediaMessage.current(message.peer, message.replyAuthor, message.replyMessage)
        ?: message.replyAttachment?.let { ChatMessage(message.replyMessage, message.replyAuthor, "", message.replyMine, "", "", false, emptyList(), emptyList(), null, true, peer = message.peer, attachment = it) } else null
    Surface(Modifier.fillMaxWidth().testTag("reply-quote"), shape = RoundedCornerShape(topStart, topEnd, 5.dp, 5.dp), color = scheme.background, contentColor = scheme.onBackground) {
        QuoteBody(name, quoteText(message.replyParts, quoted), message.replyAttachment, source, 4, Modifier.padding(horizontal = 12.dp, vertical = 10.dp), cardQuote(message.replyParts))
    }
}

// What the next message answers or replaces, above the writing field: the same block the bubble will show.
@Composable
internal fun ContextChip(title: String, text: String?, file: AttachmentDetails? = null, source: ChatMessage? = null, card: CardQuote? = null, close: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    Surface(Modifier.fillMaxWidth().padding(start = 8.dp, end = 8.dp, top = 8.dp).testTag("context-chip"), shape = RoundedCornerShape(14.dp, 14.dp, 5.dp, 5.dp), color = scheme.background, contentColor = scheme.onBackground) {
        Row(Modifier.padding(start = 12.dp, end = 4.dp, top = 2.dp, bottom = 2.dp), verticalAlignment = Alignment.CenterVertically) {
            QuoteBody(title, text, file, source, 1, Modifier.weight(1f).padding(vertical = 8.dp), card)
            Symbol("close", "Cancel reply or edit", close)
        }
    }
}
