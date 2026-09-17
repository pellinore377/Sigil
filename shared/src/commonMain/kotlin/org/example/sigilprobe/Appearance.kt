package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.ui.platform.testTag
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp

@Composable
internal fun AppearancePage(value: Appearance, analyze: (String) -> String, dynamicAvailable: Boolean, back: () -> Unit, collections: Boolean = false, setCollections: (Boolean) -> Unit = {}, collectionLabels: Boolean = true, setCollectionLabels: (Boolean) -> Unit = {}, followAccount: Boolean = true, setFollowAccount: (Boolean) -> Unit = {}, section: String = "appearance", navigate: (String) -> Unit = {}, update: (Appearance) -> Unit) {
    SettingsDetailLayout(appearanceTitle(section), back, continuous = section == "appearance" || section == "appearance-media") {
        Text(if (followAccount) "Account appearance · shared across your devices" else "Device appearance · only this device",
            Modifier.padding(horizontal = 12.dp, vertical = 16.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        when (section) {
            "appearance-objects" -> ObjectAppearance(value,update)
            "appearance-colors" -> {
                TimelinePreview(analyze, gradient = value.gradient)
                AppearanceChoices("Appearance mode", listOf("Light" to "light_mode", "Dark" to "dark_mode", "System" to "devices"), value.mode) { update(value.copy(mode = it)) }
                AccentPicker(value.accent) { update(value.copy(accent = it, dynamic = false)) }
                if (dynamicAvailable) SettingsToggle("Use Android wallpaper colors", "Follow your wallpaper accent", value.dynamic) { update(value.copy(dynamic = it)) }
                AppearanceChoices("Chat background", listOf("Solid" to "circle", "Gradient" to "gradient"), if (value.gradient) "Gradient" else "Solid") { update(value.copy(gradient = it == "Gradient")) }
                Text("Conversations follow this background unless you customize them.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            "appearance-type" -> {
                AppearanceChoices("Font family", listOf("Newsreader" to "text_format", "Google Sans Flex" to "text_format"), value.font) { update(value.copy(font = it)) }
                Text("A little room to think.", style = MaterialTheme.typography.headlineMedium)
                Text("There is something lovely about an unhurried conversation. Leave a thought, send a photograph, or make a plan for tomorrow.", style = MaterialTheme.typography.bodyLarge)
                Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceContainer) {
                    Text("let greeting = \"Hello, world\";", Modifier.fillMaxWidth().padding(16.dp), fontFamily = LocalCodeFont.current, style = MaterialTheme.typography.bodyMedium)
                }
                Text("Text size · ${(value.textScale * 100).toInt()}%", style = MaterialTheme.typography.titleMedium)
                Slider(value.textScale, { update(value.copy(textScale = it)) }, Modifier.semantics { contentDescription = "Text size" }, valueRange = .85f..1.3f, steps = 8)
                Text("Also respects your device's text-size setting. Code uses Google Sans Code with either font.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            "appearance-layout" -> {
                ChatRow(ChatSummary("preview", "Sam", "A little note for tomorrow.", "9:24am", true, emptyList()), modifier = Modifier.testTag("inbox-density-preview"), open = {})
                AppearanceChoices("Conversation spacing", listOf("Comfortable" to "view_agenda", "Compact" to "view_headline"), if (value.compact) "Compact" else "Comfortable") { update(value.copy(compact = it == "Compact")) }
                AppearanceChoices("Message previews", listOf("None" to "visibility_off", "1 line" to "short_text", "2 lines" to "notes"), listOf("None", "1 line", "2 lines")[value.previewLines]) { update(value.copy(previewLines = listOf("None", "1 line", "2 lines").indexOf(it))) }
                SettingsToggle("Collections", "Show collections in the inbox", collections, update = setCollections)
                Expandable(collections) { SettingsToggle("Show collection names", "Labels beneath collection icons", collectionLabels, update = setCollectionLabels) }
            }
            "appearance-media" -> {
                SettingsToggle("Reduce motion", "Use still transitions and indicators", value.reducedMotion) { update(value.copy(reducedMotion = it)) }
                SettingsToggle("Message effects", "Animate emoji and authored text effects", value.messageEffects) { update(value.copy(messageEffects = it)) }
                SettingsToggle("Play GIFs automatically", "Animate GIFs in the timeline", value.autoplayGifs) { update(value.copy(autoplayGifs = it)) }
                SettingsToggle("Replay message effects automatically", "", value.replaySeconds>0) {update(value.copy(replaySeconds=if(it)20 else 0))}
                Expandable(value.replaySeconds>0) {
                    Column(Modifier.padding(horizontal = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("Wait ${value.replaySeconds} seconds between replays", style = MaterialTheme.typography.titleMedium)
                        Slider(value.replaySeconds.coerceIn(10,30).toFloat(),{update(value.copy(replaySeconds=it.toInt()))},Modifier.semantics { contentDescription = "Replay interval" },valueRange=10f..30f,steps=19)
                    }
                }
                Text("Your device’s reduced-motion setting is always respected. Videos and audio play only when you choose.", Modifier.padding(12.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            else -> {
                SettingsLink("palette", "Colors & backgrounds", "Mode, accent and conversation backgrounds") { navigate("appearance-colors") }
                SettingsLink("text_format", "Typography", "${value.font} · ${(value.textScale * 100).toInt()}%") { navigate("appearance-type") }
                SettingsLink("view_agenda", "Layout", "Conversation spacing, previews and collections") { navigate("appearance-layout") }
                SettingsLink("animation", "Motion & media", "Animation, message effects and GIF playback") { navigate("appearance-media") }
                SettingsLink("casino", "Dice, coins & cards", "Materials, colors and a live playground") { navigate("appearance-objects") }
                SettingsSectionLabel("This device")
                var advanced by remember { mutableStateOf(false) }
                SigilTextButton({ advanced = !advanced }, Modifier.fillMaxWidth()) {
                    Text("Advanced", Modifier.weight(1f), textAlign = TextAlign.Start)
                    Spacer(Modifier.width(8.dp))
                    Glyph(if (advanced) "expand_less" else "expand_more", 20, if (advanced) "Hide advanced" else "Show advanced")
                }
                Expandable(advanced) { SettingsToggle("Follow account appearance on this device", "", followAccount, update = setFollowAccount) }
                SigilOutlinedButton({ update(Appearance()) }, Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp)) { Text("Reset app appearance") }
            }
        }
    }
}
internal fun appearanceTitle(section: String) = when (section) {
    "appearance-colors" -> "Colors & backgrounds"
    "appearance-type" -> "Typography"
    "appearance-layout" -> "Layout"
    "appearance-media" -> "Motion & media"
    "appearance-objects" -> "Dice, coins & cards"
    else -> "Appearance"
}

@Composable
internal fun ChatAppearance(value: ChatTheme, analyze: (String) -> String, peer: String, command: Command, back: () -> Unit, update: (ChatTheme) -> Unit) {
    var image by remember(peer) { mutableStateOf(false) }
    val gradient = value.gradient ?: LocalAppearance.current.gradient
    AppearanceLayout("Conversation appearance", "Private to you", back) {
        TimelinePreview(analyze, peer, gradient) { image = it }
        AccentPicker(value.accent) { update(value.copy(accent = it)) }
        SigilTextButton({ update(value.copy(accent = null)) }) { Text(if (value.accent == null) "Following app accent" else "Follow app accent") }
        AppearanceChoices("Background", listOf("Solid" to "circle", "Gradient" to "gradient", "Image" to "image"), if (image) "Image" else if (gradient) "Gradient" else "Solid") {
            if (it == "Image") command("attachment_pick", mapOf("peer" to peer, "kind" to "Wallpaper"))
            else { update(value.copy(gradient = it == "Gradient")); command("wallpaper_remove", mapOf("peer" to peer)) }
        }
        SigilTextButton({ update(value.copy(gradient = null)); command("wallpaper_remove", mapOf("peer" to peer)) }) { Text(if (value.gradient == null && !image) "Following app background" else "Follow app background") }
        Text("Background images stay on this device. Colors follow your account.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        SigilOutlinedButton({ update(ChatTheme()); command("wallpaper_remove", mapOf("peer" to peer)) }, Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp)) { Text("Reset conversation appearance") }
    }
}

@Composable
private fun AppearanceLayout(title: String, detail: String, back: () -> Unit, content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxSize()) {
        Header(title, back)
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
            Column(Modifier.widthIn(max = 680.dp).fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                Text(detail, Modifier.padding(horizontal = 12.dp, vertical = 16.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                content()
            }
        }
    }
}

@Composable
internal fun AppearanceChoices(label: String, choices: List<Pair<String, String>>, selected: String, update: (String) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (label.isNotBlank()) Text(label, style = MaterialTheme.typography.titleLarge)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
            choices.forEach { (name, icon) ->
                val active = selected == name
                Surface(Modifier.weight(1f).clip(RoundedCornerShape(16.dp)).selectableChoice(active) { update(name) }, shape = RoundedCornerShape(16.dp),
                    color = if (active) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
                    contentColor = if (active) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface) {
                    Row(Modifier.padding(horizontal = 8.dp, vertical = 14.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp, Alignment.CenterHorizontally)) {
                        Glyph(icon, 22); Text(name, Modifier.weight(1f, fill = false), style = MaterialTheme.typography.labelLarge, textAlign = TextAlign.Center)
                    }
                }
            }
        }
    }
}

@Composable
private fun TimelinePreview(analyze: (String) -> String, peer: String? = null, gradient: Boolean = false, hasImage: (Boolean) -> Unit = {}) {
    val scheme = MaterialTheme.colorScheme
    val messages = remember {
        listOf("Dinner still on for tonight?", "Absolutely.", "I found that little bookstore you mentioned.").mapIndexed { index, text ->
            ChatMessage("preview-$index", if (index == 0) "sam" else "self", text, index > 0, "9:24am", "Read", false, emptyList(), emptyList(), null, true)
        }
    }
    val chat = remember { ChatSummary("preview", "Sam", "", "", true, emptyList()) }
    val backdrop = rememberChromeBackdrop()
    Surface(Modifier.testTag("timeline-preview"), shape = RoundedCornerShape(24.dp), color = scheme.background) {
        Box {
        Box(Modifier.matchParentSize().captureBackdrop(backdrop)) {
        if (peer != null) { val image = LocalWallpaper.current(peer, Modifier.matchParentSize()); SideEffect { hasImage(image) } }
        if (gradient) Box(Modifier.matchParentSize().background(Brush.verticalGradient(listOf(scheme.background.copy(alpha = .7f), scheme.primaryContainer.copy(alpha = .7f)))))
        }
        Column(Modifier.padding(12.dp)) {
            FloatingChrome(backdrop, Modifier, RoundedCornerShape(24.dp)) {
                Box(Modifier.fillMaxWidth().height(72.dp)) { ConversationHeader(chat, "", false, { _, _ -> }, {}) {} }
            }
            Box(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                    Text("Today, 9:24am", Modifier.fillMaxWidth().padding(top = 6.dp, bottom = 14.dp), textAlign = TextAlign.Center, style = MaterialTheme.typography.labelMedium, color = scheme.onSurfaceVariant)
                    messages.forEachIndexed { index, message ->
                        Column(Modifier.align(if (message.mine) Alignment.End else Alignment.Start).widthIn(max = 330.dp).fillMaxWidth(.88f)
                            .padding(top = if (index == 0) 0.dp else if (index == 2) 3.dp else 12.dp), horizontalAlignment = if (message.mine) Alignment.End else Alignment.Start) {
                            MessageBubble(message, index == 2, index == 1, analyze)
                            MessageDetails(message, false, index == 2, chat, emptyMap())
                        }
                    }
                }
            }
            FloatingChrome(backdrop, Modifier.testTag("preview-footer"), RoundedCornerShape(24.dp)) {
                ComposerBar {
                    Surface(shape = RoundedCornerShape(16.dp), color = scheme.surfaceVariant) { Symbol("add", "Preview attachments") {} }
                    Composer(remember { TextFieldState() }, analyze, Modifier.weight(1f), showTools = false, enabled = false)
                    FilledIconButton({}, Modifier.size(48.dp), shape = RoundedCornerShape(16.dp), colors = IconButtonDefaults.filledIconButtonColors(containerColor = scheme.primary, contentColor = scheme.onPrimary)) { Glyph("graphic_eq", 25) }
                }
            }
        }
        }
    }
}
