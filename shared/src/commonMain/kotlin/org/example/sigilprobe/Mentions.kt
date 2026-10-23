package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.key.*
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFontFamilyResolver
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*

/** Someone a mention can name here: a full account address, how they are known, and their stable identity when known. */
data class MentionCandidate(val address: String, val name: String, val verified: Boolean = false, val identity: String? = null) {
    val handle get() = address.removePrefix("@").substringBefore(':')
}

/** Known identities, outermost first; the conversation adds its contacts to whatever a host supplies. */
val LocalMentionDirectory = staticCompositionLocalOf { emptyList<MentionCandidate>() }

internal fun mentionDirectory(chats: List<ChatSummary>, own: String): List<MentionCandidate> = chats
    .filter { !it.group && it.address != own && it.address.startsWith("@") && ':' in it.address }
    .map { MentionCandidate(it.address, it.name, it.verified, it.id.removePrefix("dm:").takeIf { _ -> it.id.startsWith("dm:") }) }

/** A written `@handle` or `@handle:server`, with its UTF-16 range. */
internal data class MentionRun(val start: Int, val end: Int, val handle: String, val server: String?) {
    val written get() = "@" + handle + (server?.let { ":$it" } ?: "")
}

private val MentionPattern = Regex("""@([A-Za-z0-9._-]+)(?::([A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+))?""")

internal fun mentionRuns(text: String): List<MentionRun> = MentionPattern.findAll(text).mapNotNull { match ->
    // Not the middle of an email address or a word.
    val before = text.getOrNull(match.range.first - 1)
    if (before != null && (before.isLetterOrDigit() || before in "_@.")) return@mapNotNull null
    val server = match.groups[2]?.value
    // Sentence punctuation after a bare handle is not part of it.
    val handle = if (server == null) match.groupValues[1].trimEnd('.', '-') else match.groupValues[1]
    if (handle.isEmpty()) null
    else MentionRun(match.range.first, if (server == null) match.range.first + 1 + handle.length else match.range.last + 1, handle, server)
}.toList()

internal sealed interface MentionResolution {
    /** The sender bound it to someone known here, or wrote the full address of someone known here. */
    data class Resolved(val candidate: MentionCandidate) : MentionResolution
    /** Not one known person: a bare handle (with whoever answers to it), or an address nobody here knows. */
    data class Unresolved(val address: String?, val candidates: List<MentionCandidate>, val bound: Boolean = false) : MentionResolution
}

/** A bare handle is never guessed: only the sender's binding or a full address settles who it means. */
internal fun resolveMention(run: MentionRun, directory: List<MentionCandidate>, binding: RichMention? = null): MentionResolution {
    val known = directory.distinctBy { it.address.lowercase() }
    if (binding != null) {
        // A known address whose identity changed is not the person the sender chose.
        val match = known.firstOrNull { it.address.equals(binding.address, ignoreCase = true) && (it.identity == null || it.identity.equals(binding.identity, ignoreCase = true)) }
        return match?.let { MentionResolution.Resolved(it) } ?: MentionResolution.Unresolved(binding.address, emptyList(), bound = true)
    }
    if (run.server != null) {
        val address = run.written
        return known.firstOrNull { it.address.equals(address, ignoreCase = true) }?.let { MentionResolution.Resolved(it) }
            ?: MentionResolution.Unresolved(address, emptyList())
    }
    return MentionResolution.Unresolved(null, known.filter { it.handle.equals(run.handle, ignoreCase = true) })
}

/** What a screen reader hears for a chip. */
internal fun mentionDescription(run: MentionRun, resolution: MentionResolution): String = when (resolution) {
    is MentionResolution.Resolved -> "Mention, ${resolution.candidate.name}" + if (resolution.candidate.verified) ", verified" else ""
    is MentionResolution.Unresolved -> when {
        resolution.candidates.size > 1 -> "Ambiguous mention ${run.written}, ${resolution.candidates.size} people"
        resolution.candidates.size == 1 -> "Unconfirmed mention ${run.written}, may be ${resolution.candidates.single().name}"
        resolution.address != null -> "Mention ${resolution.address}, not in your contacts"
        else -> "Unknown mention ${run.written}"
    }
}

/** The `@query` the caret sits at the end of, as the index of its `@` and the typed handle. */
internal fun mentionQuery(text: String, caret: Int): Pair<Int, String>? {
    if (caret < 1 || caret > text.length) return null
    var at = caret
    while (at > 0 && (text[at - 1].isLetterOrDigit() || text[at - 1] in "._-")) at--
    if (at == 0 || text[at - 1] != '@') return null
    val sign = at - 1
    if (sign > 0 && (text[sign - 1].isLetterOrDigit() || text[sign - 1] in "_@.")) return null
    return sign to text.substring(at, caret)
}

/** Handle, then a word of the name; the server part matches only once the query names one. */
internal fun mentionSuggestions(query: String, directory: List<MentionCandidate>, limit: Int = 5): List<MentionCandidate> {
    val q = query.lowercase()
    fun rank(c: MentionCandidate): Int? = when {
        ':' in q -> if (c.address.lowercase().removePrefix("@").startsWith(q)) 0 else null
        c.handle.lowercase() == q -> 0
        c.handle.lowercase().startsWith(q) -> 1
        c.name.lowercase().split(' ').any { it.startsWith(q) } -> 2
        else -> null
    }
    return directory.distinctBy { it.address.lowercase() }.mapNotNull { c -> rank(c)?.let { it to c } }
        .sortedWith(compareBy({ it.first }, { it.second.handle.lowercase() }, { it.second.address })).map { it.second }.take(limit)
}

/** The friendly handle when it names only this identity here; otherwise the full address, so the choice is never re-guessed. */
internal fun mentionInsertion(candidate: MentionCandidate, directory: List<MentionCandidate>): String =
    if (directory.count { it.handle.equals(candidate.handle, ignoreCase = true) && !it.address.equals(candidate.address, ignoreCase = true) } == 0)
        "@${candidate.handle}" else candidate.address

/** Every mention outside code and links, with who it means; a binding from the sender wins over the written text. */
internal fun mentionMarks(value: RichText, directory: List<MentionCandidate>): List<Pair<MentionRun, MentionResolution>> {
    if ('@' !in value.text) return emptyList()
    return mentionRuns(value.text).filter { run ->
        value.spans.none { it.start < run.end && run.start < it.end && ("code" in it.flags || it.link != null || it.reveal.isNotEmpty() || it.redaction > 0) }
    }.map { run -> run to resolveMention(run, directory, value.mentions.firstOrNull { it.start <= run.start && run.start < it.end }) }
}

/** The picks a draft has made, so the post carries who each handle means. */
internal class MentionChoices {
    private val picked = mutableMapOf<String, String>()
    fun pick(written: String, address: String) { if (picked.size < 64) picked[written.lowercase()] = address }
    /** One entry per written occurrence in reading order, as the core binds them; clears the draft's picks. */
    fun take(text: String): List<Map<String, String>> {
        val out = if (picked.isEmpty()) emptyList() else mentionRuns(text).mapNotNull { run ->
            val written = text.substring(run.start, run.end)
            picked[written.lowercase()]?.let { mapOf("display" to written, "address" to it) }
        }
        picked.clear()
        return out
    }
}
internal val LocalMentionChoices = staticCompositionLocalOf<MentionChoices?> { null }

/** Room around a chip, in em: added after the glyph before it and after the run, so the chip never touches a neighbour. */
private const val ChipRoom = .34f
private const val ChipPad = .18f
// Skia adds letter spacing after a glyph, Minikin half on each side; measured once so the chip edge is right on both.
private var spacingAfter: Float? = null
private fun spacingAfter(measurer: TextMeasurer): Float = spacingAfter ?: run {
    val plain = measurer.measure("xx", TextStyle(fontSize = 20.sp))
    val spaced = measurer.measure(buildAnnotatedString { withStyle(SpanStyle(letterSpacing = 1.em)) { append('x') }; append('x') }, TextStyle(fontSize = 20.sp))
    val shift = spaced.getBoundingBox(1).left - plain.getBoundingBox(1).left
    val total = spaced.size.width - plain.size.width
    (if (total > 0) shift / total else 1f).coerceIn(.3f, 1f).also { spacingAfter = it }
}

/** The display string with room reserved either side of each chip; text and offsets are unchanged, so copy is too. */
internal fun AnnotatedString.withMentionRoom(marks: List<Pair<MentionRun, MentionResolution>>, value: RichText, revealed: Set<Int>, after: Float): AnnotatedString {
    if (marks.isEmpty()) return this
    return AnnotatedString.Builder(this).apply {
        marks.forEach { (run, _) ->
            val start = motionOffsets(value, revealed, run.start)
            val end = motionOffsets(value, revealed, run.end)
            if (start in 1..length) addStyle(SpanStyle(letterSpacing = (ChipRoom / after).em), start - 1, start)
            if (after > .99f) { if (end in 1..length) addStyle(SpanStyle(letterSpacing = ChipRoom.em), end - 1, end) }
            else if (end < length) addStyle(SpanStyle(letterSpacing = (ChipRoom / (1 - after)).em), end, end + 1)
        }
    }.toAnnotatedString()
}

/** Each chip's boxes, one per line it spans, with the room trimmed off its end. */
private fun chipBoxes(marks: List<Pair<MentionRun, MentionResolution>>, value: RichText, revealed: Set<Int>, layout: TextLayoutResult, em: Float, after: Float) =
    marks.map { (run, _) ->
        val start = motionOffsets(value, revealed, run.start)
        val end = motionOffsets(value, revealed, run.end)
        if (start >= end || end > layout.layoutInput.text.length) emptyList()
        else {
            val last = layout.getLineForOffset(end - 1)
            (layout.getLineForOffset(start)..last).mapNotNull { line ->
                lineSpanEdges(layout, line, start, end)?.let { (a, b) ->
                    val base = layout.getLineBaseline(line)
                    val trim = if (line == last && after > .99f) ChipRoom * em else 0f
                    Rect(a - ChipPad * em, base - em * .92f, b - trim + ChipPad * em, base + em * .34f)
                }
            }
        }
    }

private fun TextLayoutResult.em(density: Density) = layoutInput.style.fontSize.takeIf { it.type == TextUnitType.Sp }?.let { with(density) { it.toPx() } } ?: with(density) { 16.sp.toPx() }

/** A soft chip for a mention that names one known person; a dashed ring over it for anything unsettled. */
internal fun Modifier.mentionChips(marks: List<Pair<MentionRun, MentionResolution>>, value: RichText, revealed: Set<Int>, layout: TextLayoutResult?, ink: Color, after: Float): Modifier {
    if (marks.isEmpty() || layout == null) return this
    return drawWithCache {
        val em = layout.em(this)
        val shapes = chipBoxes(marks, value, revealed, layout, em, after).zip(marks).flatMap { (boxes, mark) ->
            boxes.map { squirclePath(it, em * .30f) to (mark.second is MentionResolution.Unresolved) }
        }
        val fill = ink.copy(alpha = .12f)
        val soft = ink.copy(alpha = .06f)
        val ring = Stroke(1.dp.toPx(), pathEffect = PathEffect.dashPathEffect(floatArrayOf(3.dp.toPx(), 2.5.dp.toPx())))
        val outline = ink.copy(alpha = .55f)
        onDrawWithContent {
            shapes.forEach { (path, open) -> if (open) { drawPath(path, soft); drawPath(path, outline, style = ring) } else drawPath(path, fill) }
            drawContent()
        }
    }
}

/** Measured once per process, with the platform's own text engine. */
@Composable
internal fun mentionSpacingAfter(): Float {
    spacingAfter?.let { return it }
    return spacingAfter(TextMeasurer(LocalFontFamilyResolver.current, LocalDensity.current, LocalLayoutDirection.current, 1))
}

/** Text with mention targets laid over it; without mentions the text keeps the caller's modifier itself. */
@Composable
internal fun MentionHost(active: Boolean, modifier: Modifier, overlay: @Composable () -> Unit, text: @Composable (Modifier) -> Unit) {
    if (!active) text(modifier)
    else Box(modifier) { text(Modifier); overlay() }
}

/** Tap targets over the chips: a known person opens their details, anything else the people it could mean. */
@Composable
internal fun MentionTargets(marks: List<Pair<MentionRun, MentionResolution>>, value: RichText, revealed: Set<Int>, layout: TextLayoutResult?, after: Float) {
    if (marks.isEmpty() || layout == null) return
    val density = LocalDensity.current
    var open by remember(value) { mutableStateOf<Pair<MentionRun, MentionResolution>?>(null) }
    val em = layout.em(density)
    chipBoxes(marks, value, revealed, layout, em, after).forEachIndexed { index, boxes ->
        val mark = marks[index]
        boxes.forEachIndexed { part, box ->
            with(density) {
                Box(Modifier.offset(box.left.toDp(), box.top.toDp()).size(box.width.toDp(), box.height.toDp())
                    .clickable(role = Role.Button, onClickLabel = if (mark.second is MentionResolution.Resolved) "Show contact" else "Choose who this means") { open = mark }
                    .then(if (part == 0) Modifier.semantics { contentDescription = mentionDescription(mark.first, mark.second) } else Modifier.clearAndSetSemantics { }))
            }
        }
    }
    open?.let { (run, resolution) -> MentionSheet(run, resolution) { open = null } }
}

@Composable
private fun MentionSheet(run: MentionRun, resolution: MentionResolution, dismiss: () -> Unit) {
    var chosen by remember { mutableStateOf((resolution as? MentionResolution.Resolved)?.candidate) }
    val clipboard = LocalClipboardManager.current
    val code = MaterialTheme.typography.labelMedium.copy(fontFamily = LocalCodeFont.current)
    val person = chosen
    val unresolved = resolution as? MentionResolution.Unresolved
    val address = person?.address ?: unresolved?.address
    AlertDialog(dismiss,
        title = { Text(person?.name ?: if (unresolved?.candidates.orEmpty().isNotEmpty()) "Who is ${run.written}?" else run.written, maxLines = 2, overflow = TextOverflow.Ellipsis) },
        text = {
            // The dialog's own ink, not the bubble's the chip sat in.
            val quiet = LocalContentColor.current.copy(alpha = .68f)
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                when {
                    person != null -> {
                        Text(person.address, style = code, color = quiet)
                        if (person.verified) Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) { Glyph("verified", 18); Text("Verified", style = MaterialTheme.typography.bodyMedium) }
                        person.identity?.let { Text("Reference ${it.take(16).chunked(4).joinToString(" ")}", style = code, color = quiet) }
                        if (resolution is MentionResolution.Unresolved) Text("Your pick. The sender did not say which ${run.written} they meant.", style = MaterialTheme.typography.bodyMedium)
                    }
                    unresolved != null && unresolved.candidates.isNotEmpty() -> {
                        Text(if (unresolved.candidates.size > 1) "More than one of your contacts goes by ${run.written}. Pick the one you think it means."
                            else "The sender didn't link ${run.written} to an account. It may be:", style = MaterialTheme.typography.bodyMedium)
                        unresolved.candidates.forEach { candidate ->
                            Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(RoundedCornerShape(14.dp)).clickable(role = Role.Button) { chosen = candidate }.padding(vertical = 4.dp),
                                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                Initial(candidate.name)
                                Column(Modifier.weight(1f)) {
                                    Text(candidate.name, style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                    Text(candidate.address, style = code, color = quiet, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                }
                            }
                        }
                    }
                    address != null -> {
                        if (!address.equals(run.written, ignoreCase = true)) Text(address, style = code, color = quiet)
                        Text(if (unresolved?.bound == true) "The sender linked this account. It is not in your contacts." else "Not in your contacts.", style = MaterialTheme.typography.bodyMedium)
                    }
                    else -> Text("No one in your contacts answers to ${run.written}.", style = MaterialTheme.typography.bodyMedium)
                }
            }
        },
        confirmButton = { if (address != null) SigilTextButton({ clipboard.setText(AnnotatedString(address)); dismiss() }) { Text("Copy address") } },
        dismissButton = { SigilTextButton(dismiss) { Text("Close") } })
}

@Composable
private fun Initial(name: String) {
    Box(Modifier.size(24.dp).clip(CircleShape).background(LocalContentColor.current.copy(alpha = .10f)), contentAlignment = Alignment.Center) {
        Text(name.take(1).uppercase(), style = MaterialTheme.typography.labelMedium, maxLines = 1)
    }
}

/** Keyboard state for the picker: arrows move the highlight, Enter or Tab choose, Escape hides it until the query changes. */
@Stable
internal class MentionPickerState {
    var highlight by mutableIntStateOf(0)
    internal var dismissed by mutableStateOf<Pair<Int, String>?>(null)
    internal var options by mutableStateOf(emptyList<MentionCandidate>())
    internal var query: Pair<Int, String>? = null
    internal var choose: (MentionCandidate) -> Unit = {}
    fun onKey(event: KeyEvent): Boolean {
        if (options.isEmpty() || event.type != KeyEventType.KeyDown) return false
        when (event.key) {
            Key.DirectionDown -> highlight = (highlight + 1) % options.size
            Key.DirectionUp -> highlight = (highlight - 1 + options.size) % options.size
            Key.Enter, Key.NumPadEnter, Key.Tab -> choose(options[highlight.coerceIn(0, options.size - 1)])
            Key.Escape -> { dismissed = query; options = emptyList() }
            else -> return false
        }
        return true
    }
}

/** Suggestions for the `@` the caret is in; choosing one writes the handle, or the full address when the handle is shared. */
@Composable
internal fun MentionPicker(draft: TextFieldState, state: MentionPickerState = remember { MentionPickerState() }, modifier: Modifier = Modifier) {
    val directory = LocalMentionDirectory.current
    val choices = LocalMentionChoices.current
    val text = draft.text.toString()
    val selection = draft.selection
    // "@::" opens the contact-card picker instead.
    val typed = (if (selection.collapsed && directory.isNotEmpty()) mentionQuery(text, selection.start) else null)?.takeIf { !text.startsWith("@::", it.first) }
    LaunchedEffect(typed) { state.highlight = 0; if (typed != state.dismissed) state.dismissed = null }
    val query = typed?.takeIf { it != state.dismissed }
    val matches = remember(query?.second, directory) { query?.let { mentionSuggestions(it.second, directory) }.orEmpty() }
    SideEffect {
        state.options = matches; state.query = query
        state.choose = { candidate ->
            val now = draft.text.toString()
            mentionQuery(now, draft.selection.start)?.let { q ->
                val insert = mentionInsertion(candidate, directory)
                choices?.pick(insert, candidate.address)
                // The whole handle is replaced, including any of it after the caret.
                var end = draft.selection.start
                while (end < now.length && (now[end].isLetterOrDigit() || now[end] in "._-")) end++
                draft.edit { replace(q.first, end, "$insert "); this.selection = TextRange(q.first + insert.length + 1) }
            }
        }
    }
    if (query == null || matches.isEmpty()) return
    val ink = LocalContentColor.current
    Column(modifier.fillMaxWidth().semantics { contentDescription = "Mention suggestions" }) {
        matches.forEachIndexed { index, candidate ->
            val shared = directory.count { it.handle.equals(candidate.handle, ignoreCase = true) } > 1
            val highlighted = index == state.highlight.coerceIn(0, matches.size - 1)
            Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(RoundedCornerShape(14.dp))
                .background(if (highlighted) ink.copy(alpha = .07f) else Color.Transparent)
                .semantics { selected = highlighted }
                .clickable(role = Role.Button, onClickLabel = "Mention ${candidate.name}") { state.choose(candidate) }
                .padding(horizontal = 12.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Initial(candidate.name)
                Column(Modifier.weight(1f)) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(candidate.name, Modifier.weight(1f, fill = false), style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        if (candidate.verified) Glyph("verified", 16, "Verified")
                    }
                    // A shared handle shows the full address, the one detail that tells the two apart.
                    Text(if (shared) candidate.address else "@${candidate.handle}", style = MaterialTheme.typography.labelMedium.copy(fontFamily = LocalCodeFont.current),
                        color = ink.copy(alpha = .68f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
    }
}
