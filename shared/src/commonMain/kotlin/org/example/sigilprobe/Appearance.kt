package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp

@Composable
internal fun AppearancePage(value: Appearance, dynamicAvailable: Boolean, back: () -> Unit, collections: Boolean = false, setCollections: (Boolean) -> Unit = {}, collectionLabels: Boolean = true, setCollectionLabels: (Boolean) -> Unit = {}, followAccount: Boolean = true, setFollowAccount: (Boolean) -> Unit = {}, update: (Appearance) -> Unit) {
    AppearanceLayout("Appearance", if (followAccount) "Make Sigil feel like yours.\nThese settings follow your account." else "Make this device feel like yours.\nYour other devices keep their appearance.", back) {
        TimelinePreview()
        AppearanceChoices("Appearance mode", listOf("Light" to "light_mode", "Dark" to "dark_mode", "System" to "devices"), value.mode) { update(value.copy(mode = it)) }
        AppearanceChoices("Typography", listOf("Newsreader" to "text_format", "Google Sans Flex" to "text_format"), value.font) { update(value.copy(font = it)) }
        AccentPicker(value.accent) { update(value.copy(accent = it, dynamic = false)) }
        if (dynamicAvailable) Toggle("Use Android wallpaper colors", value.dynamic) { update(value.copy(dynamic = it)) }
        var layout by remember { mutableStateOf(false) }
        TextButton({ layout = !layout }, Modifier.fillMaxWidth()) { Text("Layout", Modifier.weight(1f), textAlign = TextAlign.Start); Glyph(if (layout) "expand_less" else "expand_more") }
        if (layout) { Toggle("Collections", collections, setCollections); if (collections) Toggle("Show collection names", collectionLabels, setCollectionLabels) }
        var advanced by remember { mutableStateOf(false) }
        TextButton({ advanced = !advanced }, Modifier.fillMaxWidth()) { Text("Advanced", Modifier.weight(1f), textAlign = TextAlign.Start); Glyph(if (advanced) "expand_less" else "expand_more") }
        if (advanced) Toggle("Follow account appearance on this device", followAccount, setFollowAccount)
        OutlinedButton({ update(Appearance()) }, Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp)) { Text("Reset app appearance") }
    }
}

@Composable
internal fun ChatAppearance(value: ChatTheme, peer: String, command: Command, back: () -> Unit, update: (ChatTheme) -> Unit) {
    var image by remember(peer) { mutableStateOf(false) }
    AppearanceLayout("Conversation appearance", "Make this conversation feel like yours.\nThese settings are private to you.", back) {
        TimelinePreview(peer, value.gradient) { image = it }
        AccentPicker(value.accent) { update(value.copy(accent = it)) }
        TextButton({ update(value.copy(accent = null)) }) { Text(if (value.accent == null) "Following app accent" else "Follow app accent") }
        AppearanceChoices("Background", listOf("Solid" to "circle", "Gradient" to "gradient", "Image" to "image"), if (image) "Image" else if (value.gradient) "Gradient" else "Solid") {
            if (it == "Image") command("attachment_pick", mapOf("peer" to peer, "kind" to "Wallpaper"))
            else { update(value.copy(gradient = it == "Gradient")); command("wallpaper_remove", mapOf("peer" to peer)) }
        }
        Text("Background images stay on this device. Colors follow your account.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        OutlinedButton({ update(ChatTheme()); command("wallpaper_remove", mapOf("peer" to peer)) }, Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp)) { Text("Reset conversation appearance") }
    }
}

@Composable
private fun AppearanceLayout(title: String, detail: String, back: () -> Unit, content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxSize()) {
        Header(title, back)
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
            Column(Modifier.widthIn(max = 680.dp).padding(20.dp), verticalArrangement = Arrangement.spacedBy(20.dp)) {
                Text(detail, Modifier.fillMaxWidth(), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center)
                content()
            }
        }
    }
}

@Composable
internal fun AppearanceChoices(label: String, choices: List<Pair<String, String>>, selected: String, update: (String) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(label, style = MaterialTheme.typography.titleLarge)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
            choices.forEach { (name, icon) ->
                val active = selected == name
                Surface(Modifier.weight(1f).selectableChoice(active) { update(name) }, shape = RoundedCornerShape(16.dp),
                    color = if (active) MaterialTheme.colorScheme.inverseSurface else MaterialTheme.colorScheme.surfaceVariant,
                    contentColor = if (active) MaterialTheme.colorScheme.inverseOnSurface else MaterialTheme.colorScheme.onSurface) {
                    Row(Modifier.padding(horizontal = 8.dp, vertical = 14.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp, Alignment.CenterHorizontally)) {
                        Glyph(icon, 22); Text(name, Modifier.weight(1f, fill = false), style = MaterialTheme.typography.labelLarge, textAlign = TextAlign.Center)
                    }
                }
            }
        }
    }
}

@Composable
private fun TimelinePreview(peer: String? = null, gradient: Boolean = false, hasImage: (Boolean) -> Unit = {}) {
    val scheme = MaterialTheme.colorScheme
    Surface(shape = RoundedCornerShape(24.dp), color = scheme.surface) {
        Column {
            Box(Modifier.fillMaxWidth().clip(RoundedCornerShape(24.dp)).background(scheme.background)) {
                if (peer != null) { val image = LocalWallpaper.current(peer, Modifier.matchParentSize()); SideEffect { hasImage(image) } }
                Column(Modifier.then(if (gradient) Modifier.background(Brush.verticalGradient(listOf(scheme.background.copy(alpha = .7f), scheme.primaryContainer.copy(alpha = .7f)))) else Modifier).padding(14.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    Text("9:24am", Modifier.fillMaxWidth(), textAlign = TextAlign.Center, style = MaterialTheme.typography.labelSmall, color = scheme.onSurfaceVariant)
                    Surface(Modifier.widthIn(max = 250.dp), shape = RoundedCornerShape(18.dp), color = scheme.surfaceVariant, contentColor = scheme.onSurfaceVariant) { Text("Dinner still on for tonight?", Modifier.padding(12.dp)) }
                    Surface(Modifier.align(Alignment.End), shape = RoundedCornerShape(18.dp, 18.dp, 4.dp, 18.dp), color = scheme.inverseSurface, contentColor = scheme.inverseOnSurface) { Text("Absolutely.", Modifier.padding(12.dp)) }
                    Surface(Modifier.align(Alignment.End).widthIn(max = 245.dp), shape = RoundedCornerShape(18.dp, 4.dp, 18.dp, 18.dp), color = scheme.inverseSurface, contentColor = scheme.inverseOnSurface) { Text("I found that little bookstore you mentioned.", Modifier.padding(12.dp)) }
                    Box(Modifier.align(Alignment.End)) { Avatar("Sam", 18) }
                }
            }
            Row(Modifier.fillMaxWidth().padding(12.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                Glyph("add")
                Surface(Modifier.weight(1f), shape = RoundedCornerShape(16.dp), color = scheme.background) { Text("Message", Modifier.padding(12.dp), color = scheme.onSurfaceVariant) }
                Surface(shape = RoundedCornerShape(14.dp), color = scheme.inverseSurface, contentColor = scheme.inverseOnSurface) { Box(Modifier.padding(12.dp)) { Glyph("graphic_eq") } }
            }
        }
    }
}
