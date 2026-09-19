package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.rememberScrollState
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.horizontalScroll
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.Velocity
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch
import sigil.shared.generated.resources.Res

// The emoji catalogue: the platform's own glyphs, named for search, grouped for the chips, and flagged when skin tones apply.
internal class EmojiEntry(val emoji: String, val name: String, val group: Int, val toned: Boolean)
internal val EmojiGroups = listOf("Smileys" to "😀", "People" to "👋", "Nature" to "🐻", "Food" to "🍎", "Travel" to "🚗", "Activities" to "⚽", "Objects" to "💡", "Symbols" to "🔣", "Flags" to "🏁")
internal val SkinTones = listOf("🏻", "🏼", "🏽", "🏾", "🏿")
internal object EmojiCatalog {
    var entries by mutableStateOf<List<EmojiEntry>>(emptyList()); private set
    val recents = mutableStateListOf<String>()
    suspend fun load() {
        if (entries.isNotEmpty()) return
        entries = Res.readBytes("files/emoji_catalog.txt").decodeToString().lineSequence().filter { it.isNotBlank() }.map { line ->
            val f = line.split('\t'); EmojiEntry(f[0], f[1], f[2].toInt(), f.getOrNull(3) == "1")
        }.toList()
    }
    fun used(emoji: String) { recents.remove(emoji); recents.add(0, emoji); while (recents.size > 24) recents.removeAt(recents.lastIndex) }
}

private const val Across = 8
private sealed class SheetRow(val group: Int) { class Cells(val cells: List<EmojiEntry>, group: Int) : SheetRow(group); class Rule(group: Int) : SheetRow(group); class Label(val text: String, group: Int) : SheetRow(group) }

// The sheet: a search field, eight-across rows between hairline rules, and category chips pinned along the bottom.
// Scrolling the rows past their top hands the overshoot to `grow`, so the surface holding the sheet can rise; `grow` returns what it took.
@Composable internal fun EmojiSheet(modifier: Modifier, grow: (Float) -> Float, settle: () -> Unit = {}, searchFocused: () -> Unit = {}, pick: (String) -> Unit) {
    LaunchedEffect(Unit) { EmojiCatalog.load() }
    val entries = EmojiCatalog.entries
    val recents = EmojiCatalog.recents
    var query by remember { mutableStateOf("") }
    var searching by remember { mutableStateOf(false) }
    val scheme = MaterialTheme.colorScheme
    val list = rememberLazyListState()
    val scope = rememberCoroutineScope()
    val growth by rememberUpdatedState(grow)
    val settling by rememberUpdatedState(settle)
    val connection = remember {
        object : NestedScrollConnection {
            override suspend fun onPreFling(available: Velocity): Velocity { settling(); return Velocity.Zero }
            override fun onPreScroll(available: Offset, source: NestedScrollSource): Offset =
                if (source == NestedScrollSource.UserInput && available.y < 0f) Offset(0f, -growth(-available.y)) else Offset.Zero
            override fun onPostScroll(consumed: Offset, available: Offset, source: NestedScrollSource): Offset =
                if (source == NestedScrollSource.UserInput && available.y > 0f) Offset(0f, -growth(-available.y)) else Offset.Zero
        }
    }
    // Rows for the sheet, and where each group starts so a chip can jump there.
    val rows = remember(entries, recents.toList(), query, searching) {
        val out = ArrayList<SheetRow>()
        val words = query.trim().lowercase().split(' ').filter { it.isNotEmpty() }
        if (words.isNotEmpty()) {
            val found = entries.filter { e -> words.all { it in e.name.lowercase() } }
            out += SheetRow.Label(if (found.isEmpty()) "Nothing named that" else "Results", -1)
            found.chunked(Across).forEach { out += SheetRow.Cells(it, -1) }
        } else {
            if (recents.isNotEmpty()) {
                if (searching) out += SheetRow.Label("Recent", -1)
                val byEmoji = entries.associateBy { it.emoji }
                recents.map { byEmoji[it] ?: EmojiEntry(it, "", -1, false) }.chunked(Across).forEach { out += SheetRow.Cells(it, -1) }
            }
            if (searching) out += SheetRow.Label("Type to search every emoji by name", -2)
            else EmojiGroups.indices.forEach { g ->
                if (out.isNotEmpty()) out += SheetRow.Rule(g)
                entries.filter { it.group == g }.chunked(Across).forEach { out += SheetRow.Cells(it, g) }
            }
        }
        out
    }
    val starts = remember(rows) { EmojiGroups.indices.map { g -> rows.indexOfFirst { it.group == g && it is SheetRow.Cells } } }
    val current by remember(rows) { derivedStateOf { rows.getOrNull(list.firstVisibleItemIndex)?.group ?: -1 } }
    val focus = LocalFocusManager.current
    var tones by remember { mutableStateOf<EmojiEntry?>(null) }
    fun choose(emoji: String) { EmojiCatalog.used(emoji); pick(emoji) }
    Column(modifier) {
        // Search: a quiet field that takes over the sheet while it has focus.
        Row(Modifier.fillMaxWidth().padding(start = 12.dp, end = 12.dp, top = 10.dp).height(40.dp).background(scheme.onSurface.copy(alpha = .07f), RoundedCornerShape(12.dp)).padding(horizontal = 10.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Glyph("search", 20, null)
            Box(Modifier.weight(1f)) {
                if (query.isEmpty()) Text("Search emoji", color = scheme.onSurface.copy(alpha = .55f), style = MaterialTheme.typography.bodyMedium)
                BasicTextField(query, { query = it }, Modifier.fillMaxWidth().onFocusChanged { if (it.isFocused) { searching = true; searchFocused() } }.semantics { contentDescription = "Search emoji" },
                    singleLine = true, textStyle = LocalTextStyle.current.copy(color = LocalContentColor.current), cursorBrush = SolidColor(LocalContentColor.current), keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search))
            }
            if (searching || query.isNotEmpty()) Box(Modifier.clip(RoundedCornerShape(8.dp)).clickable { query = ""; searching = false; focus.clearFocus() }.padding(2.dp)) { Glyph("close", 20, "Leave search") }
        }
        LazyColumn(Modifier.weight(1f).fillMaxWidth().nestedScroll(connection).padding(horizontal = 10.dp), list, contentPadding = PaddingValues(top = 8.dp, bottom = 8.dp)) {
            itemsIndexed(rows) { _, row ->
                when (row) {
                    is SheetRow.Rule -> Box(Modifier.fillMaxWidth().padding(horizontal = 4.dp, vertical = 6.dp).height(1.dp).background(scheme.onSurface.copy(alpha = .10f)))
                    is SheetRow.Label -> Text(row.text, Modifier.fillMaxWidth().padding(horizontal = 6.dp, vertical = if (row.group == -2) 12.dp else 6.dp), style = MaterialTheme.typography.labelMedium, color = scheme.onSurface.copy(alpha = .6f),
                        textAlign = if (row.group == -2) androidx.compose.ui.text.style.TextAlign.Center else null)
                    is SheetRow.Cells -> Row(Modifier.fillMaxWidth()) {
                        row.cells.forEach { e ->
                            Box(Modifier.weight(1f).aspectRatio(1f).clip(RoundedCornerShape(10.dp)).combinedClickable(onClick = { choose(e.emoji) }, onLongClick = { if (e.toned) tones = e }).semantics { contentDescription = e.name.ifEmpty { e.emoji } }, contentAlignment = Alignment.Center) {
                                Text(e.emoji, fontSize = 24.sp, maxLines = 1, softWrap = false)
                                // Skin tones for a held emoji, offered in a small row.
                                if (tones === e) DropdownMenu(true, { tones = null }, shape = RoundedCornerShape(16.dp)) {
                                    Row(Modifier.padding(horizontal = 4.dp)) {
                                        (listOf("") + SkinTones).forEach { t ->
                                            val toned = if (t.isEmpty()) e.emoji else e.emoji.replace("️", "") + t
                                            Text(toned, Modifier.clip(RoundedCornerShape(10.dp)).clickable { tones = null; choose(toned) }.padding(8.dp), fontSize = 24.sp, maxLines = 1, softWrap = false)
                                        }
                                    }
                                }
                            }
                        }
                        repeat(Across - row.cells.size) { Spacer(Modifier.weight(1f)) }
                    }
                }
            }
        }
        // Category chips along the bottom; the current one carries its name.
        if (!searching && query.isEmpty()) Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(start = 10.dp, end = 10.dp, top = 8.dp, bottom = 10.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            val chips = (if (recents.isNotEmpty()) listOf(-1 to ("Recent" to "🕘")) else emptyList()) + EmojiGroups.mapIndexed { i, g -> i to g }
            chips.forEach { (g, chip) ->
                val on = current == g
                Row(Modifier.height(32.dp).clip(RoundedCornerShape(10.dp)).background(if (on) scheme.primary else scheme.onSurface.copy(alpha = .07f))
                    .clickable { scope.launch { list.animateScrollToItem(if (g < 0) 0 else starts.getOrElse(g) { 0 }.coerceAtLeast(0)) } }.padding(horizontal = if (on) 10.dp else 8.dp).semantics { contentDescription = chip.first },
                    verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(chip.second, fontSize = 16.sp, maxLines = 1, softWrap = false)
                    if (on) Text(chip.first, style = MaterialTheme.typography.labelMedium, color = scheme.onPrimary)
                }
            }
        }
    }
}

// The reaction drawer: frosted glass rising over the composer, peeking first and growing to a near-full page as its sheet scrolls.
@Composable internal fun EmojiDrawer(close: () -> Unit, pick: (String) -> Unit) {
    val host = LocalFooterHost.current
    val chrome = host?.chrome
    Presented(close) {
        BoxWithConstraints(Modifier.fillMaxSize()) {
            val density = LocalDensity.current
            val ime = with(density) { WindowInsets.ime.getBottom(density).toDp() }
            var top by remember { mutableStateOf(0f) }
            // The drawer stops one gap beneath the header, like every other panel.
            val headerBottom = host?.headerBottom ?: 0f
            val ceiling = if (headerBottom > top) maxHeight - with(density) { (headerBottom - top).toDp() } - 12.dp else maxHeight * .92f
            val peek = maxHeight * .46f
            val full = minOf(maxHeight * .92f, ceiling)
            var extra by remember { mutableStateOf(0f) }
            val scope = rememberCoroutineScope()
            val reach = with(density) { (full - peek).coerceAtLeast(0.dp).toPx() }
            fun settle() { val target = if (extra > reach / 2f) reach else 0f; scope.launch { androidx.compose.animation.core.animate(extra, target) { value, _ -> extra = value } } }
            val height = minOf(peek + with(density) { extra.toDp() }, full, maxHeight - ime - 24.dp).coerceAtLeast(120.dp)
            Box(Modifier.fillMaxSize().onGloballyPositioned { top = it.positionInWindow().y }.background(MaterialTheme.colorScheme.scrim.copy(alpha = .42f)).clickable(remember { MutableInteractionSource() }, null) { close() })
            // The same clamp in layout: the drawer's top never passes the header while the keyboard is still moving.
            val imeInsets = WindowInsets.ime
            FloatingChrome(chrome, Modifier.align(Alignment.BottomCenter).widthIn(max = 920.dp).fillMaxWidth().padding(horizontal = 6.dp).imePadding().height(height).layout { measurable, constraints ->
                val limit = if (headerBottom > top) (maxHeight.roundToPx() - imeInsets.getBottom(this) - (headerBottom - top).toInt() - 12.dp.roundToPx()).coerceAtLeast(0) else constraints.maxHeight
                val placeable = measurable.measure(constraints.copy(maxHeight = minOf(constraints.maxHeight, limit), minHeight = minOf(constraints.minHeight, limit)))
                layout(placeable.width, placeable.height) { placeable.place(0, 0) }
            }, RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp)) {
                Column(Modifier.fillMaxSize().navigationBarsPadding()) {
                    // The handle drags the drawer: up to grow, down to shrink, and further down past its peek to let it go.
                    Box(Modifier.fillMaxWidth().height(24.dp).pointerInput(Unit) {
                        detectVerticalDragGestures(
                            onDragEnd = { if (extra < with(density) { (-72).dp.toPx() }) close() else settle() },
                            onDragCancel = { settle() }) { change, drag ->
                            extra = (extra - drag).coerceIn(with(density) { -(peek - 96.dp).toPx() }, reach); change.consume()
                        }
                    }, contentAlignment = Alignment.Center) { Box(Modifier.size(32.dp, 4.dp).background(MaterialTheme.colorScheme.onSurface.copy(alpha = .35f), RoundedCornerShape(2.dp))) }
                    EmojiSheet(Modifier.fillMaxSize(), grow = { delta ->
                        val next = (extra + delta).coerceIn(0f, reach)
                        val applied = next - extra; extra = next; applied
                    }, settle = ::settle) { pick(it); close() }
                }
            }
        }
    }
}
