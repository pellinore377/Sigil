package org.sigil

import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.sp

// What a file is for the bubble it lands in; the chip is the reference's type badge.
enum class AttachmentKind(val chip: String, val glyph: String) {
    Markdown("MD", "description"), Text("TXT", "description"), Sheet("SHEET", "table_chart"), Pdf("PDF", "picture_as_pdf"),
    Document("DOC", "description"), Slides("SLIDES", "slideshow"), Audio("AUDIO", "music_note"), File("FILE", "draft")
}

fun attachmentKind(name: String, mediaType: String): AttachmentKind = when (name.substringAfterLast('.', "").lowercase()) {
    "md", "markdown" -> AttachmentKind.Markdown
    "txt", "log", "json", "jsonl", "xml", "yaml", "yml", "toml", "rs", "c", "cpp", "h", "html", "css", "kt", "py", "js", "ts" -> AttachmentKind.Text
    "csv", "tsv", "xls", "xlsx", "xlsb", "ods" -> AttachmentKind.Sheet
    "pdf" -> AttachmentKind.Pdf
    "doc", "docx", "odt", "rtf" -> AttachmentKind.Document
    "ppt", "pptx", "odp" -> AttachmentKind.Slides
    "mp3", "m4a", "aac", "flac", "ogg", "opus", "wav", "wma" -> AttachmentKind.Audio
    else -> when {
        mediaType == "application/pdf" -> AttachmentKind.Pdf
        mediaType.startsWith("audio/") -> AttachmentKind.Audio
        mediaType == "text/markdown" -> AttachmentKind.Markdown
        mediaType.startsWith("text/") -> AttachmentKind.Text
        else -> AttachmentKind.File
    }
}

// The glimpse a card shows before the file is opened.
sealed interface FilePeek {
    data class Text(val text: String, val markdown: Boolean) : FilePeek
    data class Table(val cells: List<List<String>>) : FilePeek
    data class Page(val image: ImageBitmap) : FilePeek
    data class Track(val tags: TrackTags?, val art: ImageBitmap?, val durationMs: Long?) : FilePeek
}

// Each platform reads the file's head its own way; null means the card shows only what the message already says.
val LocalFilePeek = staticCompositionLocalOf<(suspend (ChatMessage) -> FilePeek?)?> { null }

expect fun decodeImage(bytes: ByteArray): ImageBitmap?

fun attachmentSize(bytes: Long): String = when {
    bytes >= 1024L * 1024 * 1024 -> "${(bytes * 10 / (1024L * 1024 * 1024)) / 10.0} GB"
    bytes >= 1024L * 1024 -> "${(bytes * 10 / (1024L * 1024)) / 10.0} MB"
    else -> "${(bytes * 10 / 1024) / 10.0} KB"
}

fun trackTime(ms: Long): String {
    val seconds = (ms / 1000).coerceAtLeast(0)
    val minutes = seconds / 60
    val rest = seconds % 60
    return if (minutes >= 60) "${minutes / 60}:${(minutes % 60).toString().padStart(2, '0')}:${rest.toString().padStart(2, '0')}" else "$minutes:${rest.toString().padStart(2, '0')}"
}

// RFC 4180 cells, bounded; a quoted field may hold the delimiter and doubled quotes.
fun parseDelimited(text: String, delimiter: Char, maxRows: Int = 64, maxColumns: Int = 32): List<List<String>> {
    val rows = ArrayList<List<String>>()
    var row = ArrayList<String>(); val cell = StringBuilder(); var quoted = false
    var i = 0
    fun endCell() { if (row.size < maxColumns) row.add(cell.toString()); cell.setLength(0) }
    fun endRow() { endCell(); rows.add(row); row = ArrayList() }
    while (i < text.length && rows.size < maxRows) {
        val c = text[i]
        when {
            quoted && c == '"' && i + 1 < text.length && text[i + 1] == '"' -> { cell.append('"'); i++ }
            quoted && c == '"' -> quoted = false
            !quoted && c == '"' && cell.isEmpty() -> quoted = true
            !quoted && c == delimiter -> endCell()
            !quoted && c == '\n' -> endRow()
            !quoted && c == '\r' -> Unit
            else -> cell.append(c)
        }
        i++
    }
    if (rows.size < maxRows && (cell.isNotEmpty() || row.isNotEmpty())) endRow()
    return rows
}

// The most present saturated hue of a picture, sampled sparsely; grey pictures yield their mid tone.
fun dominantColor(image: ImageBitmap): Color? {
    if (image.width < 1 || image.height < 1) return null
    val pixels = image.toPixelMap()
    val step = maxOf(1, minOf(image.width, image.height) / 48)
    val buckets = HashMap<Int, IntArray>()
    var y = 0
    while (y < image.height) {
        var x = 0
        while (x < image.width) {
            val p = pixels[x, y]
            if (p.alpha > .5f) {
                val r = (p.red * 255).toInt(); val g = (p.green * 255).toInt(); val b = (p.blue * 255).toInt()
                val max = maxOf(r, g, b); val min = minOf(r, g, b)
                val saturation = if (max == 0) 0 else (max - min) * 255 / max
                val weight = 1 + saturation / 32 + (if (max in 40..230) 2 else 0)
                val key = (r shr 5 shl 6) or (g shr 5 shl 3) or (b shr 5)
                val bucket = buckets.getOrPut(key) { IntArray(4) }
                bucket[0] += weight; bucket[1] += r * weight; bucket[2] += g * weight; bucket[3] += b * weight
            }
            x += step
        }
        y += step
    }
    val best = buckets.values.maxByOrNull { it[0] } ?: return null
    return Color(best[1] / best[0] / 255f, best[2] / best[0] / 255f, best[3] / best[0] / 255f)
}

fun Color.brightness(): Float = .2126f * red + .7152f * green + .0722f * blue

// Enough Markdown for a page to read as one: headings, emphasis, bullets, code spans and rules.
fun markdownPreview(text: String, scale: Float = 1f): AnnotatedString = buildAnnotatedString {
    val lines = text.split('\n')
    var fenced = false
    for ((index, raw) in lines.withIndex()) {
        val line = raw.trimEnd('\r')
        if (line.trimStart().startsWith("```")) { fenced = !fenced; continue }
        if (index > 0 && length > 0) append('\n')
        if (fenced) { withStyle(SpanStyle(fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace)) { append(line) }; continue }
        val trimmed = line.trimStart()
        val heading = trimmed.takeWhile { it == '#' }.length
        when {
            heading in 1..6 && trimmed.getOrNull(heading) == ' ' -> withStyle(SpanStyle(fontWeight = FontWeight.Bold, fontSize = (if (heading == 1) 16f else if (heading == 2) 14f else 13f).times(scale).sp)) { inlineMarkdown(trimmed.drop(heading + 1)) }
            trimmed.startsWith("- ") || trimmed.startsWith("* ") || trimmed.startsWith("+ ") -> { append("  • "); inlineMarkdown(trimmed.drop(2)) }
            Regex("""^\d+[.)] """).containsMatchIn(trimmed) -> { append("  "); inlineMarkdown(trimmed) }
            trimmed.startsWith("> ") -> withStyle(SpanStyle(fontStyle = androidx.compose.ui.text.font.FontStyle.Italic)) { inlineMarkdown(trimmed.drop(2)) }
            trimmed == "---" || trimmed == "***" -> append("──────")
            else -> inlineMarkdown(line)
        }
    }
}

private fun androidx.compose.ui.text.AnnotatedString.Builder.inlineMarkdown(text: String) {
    var i = 0
    while (i < text.length) {
        val rest = text.substring(i)
        val bold = Regex("""^\*\*(.+?)\*\*""").find(rest) ?: Regex("""^__(.+?)__""").find(rest)
        val code = Regex("""^`([^`]+)`""").find(rest)
        val italic = Regex("""^\*(?!\*)(.+?)\*""").find(rest) ?: Regex("""^_(?!_)(.+?)_""").find(rest)
        val link = Regex("""^\[([^\]]+)]\([^)]*\)""").find(rest)
        when {
            bold != null -> { withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append(bold.groupValues[1]) }; i += bold.value.length }
            code != null -> { withStyle(SpanStyle(fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace)) { append(code.groupValues[1]) }; i += code.value.length }
            italic != null -> { withStyle(SpanStyle(fontStyle = androidx.compose.ui.text.font.FontStyle.Italic)) { append(italic.groupValues[1]) }; i += italic.value.length }
            link != null -> { withStyle(SpanStyle(textDecoration = androidx.compose.ui.text.style.TextDecoration.Underline)) { append(link.groupValues[1]) }; i += link.value.length }
            else -> { append(text[i]); i++ }
        }
    }
}
