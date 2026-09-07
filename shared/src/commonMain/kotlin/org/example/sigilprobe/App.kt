package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.text.input.clearText
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.jetbrains.compose.resources.Font
import sigil.shared.generated.resources.*

private data class Letter(val text: String, val mine: Boolean)
private data class Conversation(val id: String, val name: String, val initial: String, val preview: String)
private val examples = listOf(
    Conversation("alex", "Alex Morgan", "A", "A little room to think."),
    Conversation("weekend", "Weekend plans", "W", "Shall we take the long way home?"),
    Conversation("self", "Note to Self", "S", "Small ideas, kept close."),
)

@Composable
fun SigilApp(palette: (Int, Boolean) -> String, analyze: (String) -> String, redact: (String) -> String,
    read: (String) -> String? = { null }, write: (String, String) -> Unit = { _, _ -> }, dynamicAccent: Int? = null,
    onBackAvailable: (Boolean, () -> Unit) -> Unit = { _, _ -> }) {
    var appearance by remember { mutableStateOf(decodeAppearance(read("appearance"))) }
    val themes = remember { mutableStateMapOf<String, ChatTheme>().apply { examples.forEach { put(it.id, decodeChat(read("chat.${it.id}"))) } } }
    val letters = remember { examples.associate { it.id to mutableStateListOf(Letter(it.preview, false)) } }
    val drafts = remember { examples.associate { it.id to TextFieldState() } }
    var selected by remember { mutableStateOf<String?>(null) }
    var settings by remember { mutableStateOf(false) }
    var chatSettings by remember { mutableStateOf(false) }
    val back = { if (settings) settings = false else if (chatSettings) chatSettings = false else selected = null }
    SideEffect { onBackAvailable(settings || selected != null, back) }
    SigilTheme(appearance, dynamicAccent = dynamicAccent, palette = palette) {
        Surface(color = MaterialTheme.colorScheme.background) {
            BoxWithConstraints(Modifier.fillMaxSize().safeDrawingPadding().imePadding()) {
                val wide = maxWidth >= 840.dp
                if (settings) {
                    AppearancePage(appearance, dynamicAccent != null, back) {
                        appearance = it; write("appearance", it.encode())
                    }
                } else Row(Modifier.fillMaxSize()) {
                    if (selected == null || wide) Column(Modifier.then(if (wide) Modifier.width(320.dp) else Modifier.fillMaxWidth()).fillMaxHeight()) {
                        Inbox(selected, { selected = it; chatSettings = false }, { settings = true })
                    }
                    if (wide) VerticalDivider()
                    val conversation = examples.find { it.id == selected }
                    if (conversation != null) {
                        val theme = themes.getValue(conversation.id)
                        SigilTheme(appearance, theme, dynamicAccent, palette) {
                            Surface(Modifier.weight(1f).fillMaxHeight(), color = MaterialTheme.colorScheme.background) {
                                if (chatSettings) ChatAppearance(theme, { chatSettings = false }) {
                                    themes[conversation.id] = it; write("chat.${conversation.id}", it.encode())
                                } else ConversationPage(conversation, letters.getValue(conversation.id), drafts.getValue(conversation.id),
                                    theme.gradient, back, { chatSettings = true }, analyze, redact)
                            }
                        }
                    } else if (wide) Box(Modifier.weight(1f).fillMaxHeight(), contentAlignment = Alignment.Center) {
                        Text("A little room for correspondence.", style = MaterialTheme.typography.headlineSmall)
                    }
                }
            }
        }
    }
}

@Composable
private fun Symbol(name: String, label: String, action: () -> Unit) {
    IconButton(action, Modifier.semantics { contentDescription = label }) {
        Text(name, fontFamily = FontFamily(Font(Res.font.material_symbols)), fontSize = 24.sp,
            modifier = Modifier.clearAndSetSemantics { })
    }
}

@Composable
private fun Header(title: String, back: (() -> Unit)? = null, action: @Composable RowScope.() -> Unit = {}) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        if (back != null) Symbol("arrow_back", "Back", back)
        Text(title, Modifier.weight(1f).padding(start = 8.dp), style = MaterialTheme.typography.headlineMedium)
        action()
    }
}

@Composable
private fun Inbox(selected: String?, open: (String) -> Unit, settings: () -> Unit) {
    var search by remember { mutableStateOf("") }
    Column(Modifier.fillMaxHeight()) {
        Header("Sigil") { Symbol("settings", "Appearance", settings) }
        OutlinedTextField(search, { search = it }, Modifier.fillMaxWidth().padding(horizontal = 20.dp),
            label = { Text("Search conversations") }, singleLine = true, shape = RoundedCornerShape(18.dp))
        Text("Development · sample conversations", Modifier.padding(20.dp), style = MaterialTheme.typography.bodySmall)
        LazyColumn(Modifier.weight(1f)) {
            items(examples.filter { it.name.contains(search, ignoreCase = true) }, key = { it.id }) { conversation ->
                val active = selected == conversation.id
                Row(Modifier.fillMaxWidth().clickable { open(conversation.id) }
                    .background(if (active) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.background)
                    .padding(horizontal = 20.dp, vertical = 20.dp), verticalAlignment = Alignment.CenterVertically) {
                    Surface(Modifier.size(48.dp), shape = CircleShape, color = MaterialTheme.colorScheme.surfaceVariant) {
                        Box(contentAlignment = Alignment.Center) { Text(conversation.initial, style = MaterialTheme.typography.titleLarge) }
                    }
                    Column(Modifier.weight(1f).padding(start = 16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(conversation.name, style = MaterialTheme.typography.titleLarge)
                        Text(conversation.preview, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium)
                    }
                }
                HorizontalDivider(Modifier.padding(start = 84.dp, end = 20.dp), color = MaterialTheme.colorScheme.outlineVariant)
            }
        }
    }
}

@Composable
private fun ConversationPage(conversation: Conversation, letters: MutableList<Letter>, draft: TextFieldState,
    gradient: Boolean, back: () -> Unit, settings: () -> Unit, analyze: (String) -> String, redact: (String) -> String) {
    var formatting by remember { mutableStateOf(false) }
    val scheme = MaterialTheme.colorScheme
    Column(Modifier.fillMaxSize().then(if (gradient) Modifier.background(Brush.verticalGradient(listOf(scheme.background, scheme.primaryContainer))) else Modifier)) {
        Header(conversation.name, back) { Symbol("tune", "Conversation appearance", settings) }
        Text("Local preview · messages stay in this session", Modifier.padding(horizontal = 20.dp), style = MaterialTheme.typography.bodySmall)
        LazyColumn(Modifier.weight(1f).fillMaxWidth().padding(horizontal = 20.dp), reverseLayout = true,
            contentPadding = PaddingValues(vertical = 24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            items(letters.asReversed()) { letter ->
                Row(Modifier.fillMaxWidth(), horizontalArrangement = if (letter.mine) Arrangement.End else Arrangement.Start) {
                    Surface(Modifier.widthIn(max = 520.dp).fillMaxWidth(.88f), shape = RoundedCornerShape(18.dp),
                        color = if (letter.mine) scheme.primaryContainer else scheme.surface,
                        contentColor = if (letter.mine) scheme.onPrimaryContainer else scheme.onSurface) {
                        Text(letter.text, Modifier.padding(16.dp), style = MaterialTheme.typography.bodyLarge)
                    }
                }
            }
        }
        Surface(color = scheme.surface) {
            Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                Composer(draft, analyze, showTools = formatting)
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    TextButton({ formatting = !formatting }) { Text(if (formatting) "Hide formatting" else "Formatting") }
                    Spacer(Modifier.weight(1f))
                    Button({
                        val text = redact(draft.text.toString())
                        if (text != "Invalid SigilText; send blocked") { letters.add(Letter(text, true)); draft.clearText() }
                    }, enabled = draft.text.isNotBlank() && redact(draft.text.toString()) != "Invalid SigilText; send blocked") { Text("Send locally") }
                }
            }
        }
    }
}

@Composable
private fun AppearancePage(value: Appearance, dynamicAvailable: Boolean, back: () -> Unit, update: (Appearance) -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
        Header("Appearance", back)
        Column(Modifier.widthIn(max = 680.dp).padding(horizontal = 24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text("Your ink. Your paper.", style = MaterialTheme.typography.headlineLarge)
            Text("Defaults for Sigil and conversations that follow the app theme.")
            Choices("Typeface", listOf("Newsreader", "Google Sans Flex"), value.font) { update(value.copy(font = it)) }
            Choices("Appearance mode", listOf("System", "Light", "Dark"), value.mode) { update(value.copy(mode = it)) }
            if (dynamicAvailable) Toggle("Use Android wallpaper colors", value.dynamic) { update(value.copy(dynamic = it)) }
            AccentPicker(value.accent) { update(value.copy(accent = it, dynamic = false)) }
            Preview()
            TextButton({ update(Appearance()) }) { Text("Reset app appearance") }
        }
    }
}

@Composable
private fun ChatAppearance(value: ChatTheme, back: () -> Unit, update: (ChatTheme) -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
        Header("Chat appearance", back)
        Column(Modifier.widthIn(max = 680.dp).padding(horizontal = 24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text("Only for you", style = MaterialTheme.typography.headlineLarge)
            Text("Your accent colors the whole conversation. Other participants keep their own theme.")
            Text(if (value.accent == null) "Accent follows the app" else "Custom conversation accent", style = MaterialTheme.typography.bodyMedium)
            AccentPicker(value.accent) { update(value.copy(accent = it)) }
            TextButton({ update(value.copy(accent = null)) }) { Text("Follow app accent") }
            Toggle("Gradient background", value.gradient) { update(value.copy(gradient = it)) }
            Preview()
            TextButton({ update(ChatTheme()) }) { Text("Reset conversation appearance") }
        }
    }
}

@Composable
private fun Choices(label: String, choices: List<String>, selected: String, update: (String) -> Unit) {
    Column {
        Text(label, style = MaterialTheme.typography.titleMedium)
        choices.forEach { choice ->
            Row(Modifier.fillMaxWidth().selectableChoice(choice == selected) { update(choice) }, verticalAlignment = Alignment.CenterVertically) {
                RadioButton(choice == selected, null)
                Text(choice, Modifier.padding(12.dp))
            }
        }
    }
}
private fun Modifier.selectableChoice(selected: Boolean, action: () -> Unit) = this
    .semantics { this.selected = selected; role = Role.RadioButton }.clickable(onClick = action).heightIn(min = 48.dp)

@Composable
private fun Toggle(label: String, checked: Boolean, update: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, Modifier.weight(1f))
        Switch(checked, update, Modifier.semantics { contentDescription = label })
    }
}

@Composable
private fun AccentPicker(value: Int?, update: (Int) -> Unit) {
    var text by remember(value) { mutableStateOf(value?.let(::accentText) ?: "") }
    val parsed = parseAccent(text)
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("Accent color", style = MaterialTheme.typography.titleMedium)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf(0x555555, 0x48658c, 0x8b4861, 0x806333, 0x586951).forEach { color ->
                Box(Modifier.size(48.dp).background(androidx.compose.ui.graphics.Color(0xff000000L or color.toLong()), CircleShape)
                    .semantics { contentDescription = "Accent ${accentText(color)}"; selected = value == color }
                    .clickable { update(color) }, contentAlignment = Alignment.Center) {
                    if (value == color) Text("✓", color = androidx.compose.ui.graphics.Color.White)
                }
            }
        }
        OutlinedTextField(text, { text = it.take(7) }, label = { Text("Hex color") }, prefix = { Text("#") },
            singleLine = true, isError = text.isNotEmpty() && parsed == null,
            supportingText = { Text("Six hexadecimal digits, for example 48658C") })
        TextButton({ parsed?.let(update) }, enabled = parsed != null) { Text("Apply accent") }
    }
}

@Composable
private fun Preview() {
    Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.background, RoundedCornerShape(20.dp)).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text("A little correspondence", style = MaterialTheme.typography.titleLarge)
        Surface(shape = RoundedCornerShape(14.dp), color = MaterialTheme.colorScheme.surface) { Text("Room for a thought.", Modifier.padding(14.dp)) }
        Surface(Modifier.align(Alignment.End), shape = RoundedCornerShape(14.dp), color = MaterialTheme.colorScheme.primaryContainer,
            contentColor = MaterialTheme.colorScheme.onPrimaryContainer) { Text("And a thoughtful reply.", Modifier.padding(14.dp)) }
        Text("let thought = \"hello\";", fontFamily = LocalCodeFont.current, style = MaterialTheme.typography.bodyMedium)
    }
}
