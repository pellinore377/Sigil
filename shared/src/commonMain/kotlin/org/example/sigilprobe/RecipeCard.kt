package org.sigil

import androidx.compose.animation.*
import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*
import kotlinx.coroutines.CancellationException
import kotlin.math.abs

@Composable
internal fun RecipeCard(message: ChatMessage, part: MessagePart) {
    val original = part.recipe ?: return
    var expanded by remember(original) { mutableStateOf(false) }
    var checked by remember(original) { mutableStateOf(emptySet<Int>()) }
    var step by remember(original) { mutableIntStateOf(0) }
    var ingredients by remember(original) { mutableStateOf(true) }
    var requested by remember(original) { mutableStateOf(original.serves) }
    var value by remember(original) { mutableStateOf(original) }
    var issue by remember { mutableStateOf(false) }
    var loading by remember { mutableStateOf(false) }
    var attempt by remember { mutableIntStateOf(0) }
    var awake by remember { mutableStateOf(false) }
    val scale = LocalRecipeScale.current
    val keepAwake = LocalKeepScreenAwake.current
    val motion = LocalMotion.current
    LaunchedEffect(original, requested, attempt) {
        val target = requested
        if (target == null || target == original.serves) { value = original; issue = false; loading = false; return@LaunchedEffect }
        if (scale == null) return@LaunchedEffect
        loading = true; issue = false
        try { value = scale(message, part, target) }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { issue = true }
        finally { loading = false }
    }
    @Composable fun metadata(value: RecipeContent) {
        val pieces = listOfNotNull(value.serves?.let { "Serves $it" }, value.seconds?.let { if (it < 60) "$it sec" else if (it % 60 == 0L) "${it / 60} min" else "${it / 60} min ${it % 60} sec" })
        if (pieces.isNotEmpty()) Text(pieces.joinToString(" · "), style = MaterialTheme.typography.labelMedium)
    }
    Column(Modifier.widthIn(min = 200.dp, max = 280.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) { Glyph("skillet", 20); Text("Recipe", style = MaterialTheme.typography.labelMedium) }
        RichMessageText(original.title, style = MaterialTheme.typography.titleMedium)
        metadata(original)
        original.ingredients.take(3).forEach { RichMessageText(it, Modifier.heightIn(max = 64.dp).clipToBounds(), MaterialTheme.typography.bodyMedium) }
        original.steps.firstOrNull()?.let { Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) { Text("1.", style = MaterialTheme.typography.bodyMedium); RichMessageText(it, Modifier.weight(1f).heightIn(max = 72.dp).clipToBounds(), MaterialTheme.typography.bodyMedium) } }
        Text("${original.ingredients.size} ingredients · ${original.steps.size} steps", style = MaterialTheme.typography.labelSmall)
        SigilTextButton({ expanded = true }) { Glyph("open_in_full", 18); Spacer(Modifier.width(8.dp)); Text("Open recipe") }
    }
    if (expanded) Dialog({ expanded = false; awake = false }, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface) {
                keepAwake?.invoke(awake)
                Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        SigilIconButton({ expanded = false; awake = false }) { Glyph("close", 24, "Close recipe") }
                        Text("Recipe", Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
                    }
                    RichMessageText(original.title, Modifier.heightIn(max = 120.dp).verticalScroll(rememberScrollState()), MaterialTheme.typography.titleLarge)
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Column(Modifier.weight(1f)) { metadata(value) }
                        if (original.serves != null && scale != null) {
                            SigilIconButton({ requested = (value.serves ?: original.serves) - 1; attempt++ }, enabled = !loading && (value.serves ?: 0) > 1) { Glyph("remove", 20, "Fewer servings") }
                            SigilIconButton({ requested = (value.serves ?: original.serves) + 1; attempt++ }, enabled = !loading && (value.serves ?: 65535) < 65535) { Glyph("add", 20, "More servings") }
                        }
                    }
                    if (loading) Text("Adjusting servings…", style = MaterialTheme.typography.labelMedium)
                    if (issue) Text("Couldn't adjust servings. The previous amounts are still shown.", style = MaterialTheme.typography.bodySmall)
                    if (value.serves != original.serves) Text("Adjusted amounts are marked. Other quantities remain as written.", style = MaterialTheme.typography.bodySmall)
                    AppearanceChoices("Cooking view", listOf("Ingredients" to "grocery", "Steps" to "format_list_numbered"), if (ingredients) "Ingredients" else "Steps") { ingredients = it == "Ingredients" }
                    if (ingredients) LazyColumn(Modifier.weight(1f).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        itemsIndexed(value.ingredients, key = { index, _ -> index }) { index, text ->
                            val done = index in checked
                            Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).background(MaterialTheme.colorScheme.surfaceVariant, MaterialTheme.shapes.medium)
                                .toggleable(done, role = Role.Checkbox) { checked = if (done) checked - index else checked + index }.padding(12.dp),
                                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                Glyph(if (done) "check_box" else "check_box_outline_blank", 24)
                                Column(Modifier.weight(1f).alpha(if (done) .6f else 1f)) {
                                    RichMessageText(text)
                                    if (value.scaled.getOrNull(index) == true) Text("Adjusted", style = MaterialTheme.typography.labelSmall)
                                }
                            }
                        }
                    } else {
                        AnimatedContent(step, Modifier.weight(1f).fillMaxWidth(), transitionSpec = {
                            (slideInHorizontally(motion.tween(180)) { if (targetState > initialState) it else -it } + fadeIn(motion.tween(180))) togetherWith
                                (slideOutHorizontally(motion.tween(180)) { if (targetState > initialState) -it else it } + fadeOut(motion.tween(180)))
                        }, label = "recipe-step") { index ->
                            Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surfaceVariant, MaterialTheme.shapes.large).pointerInput(index) {
                                var distance = 0f
                                detectHorizontalDragGestures(onDragStart = { distance = 0f }, onDragCancel = { distance = 0f }, onDragEnd = {
                                    if (abs(distance) >= 64.dp.toPx()) step = (index + if (distance < 0) 1 else -1).coerceIn(0, original.steps.lastIndex)
                                }) { change, delta -> change.consume(); distance += delta }
                            }.verticalScroll(rememberScrollState()).padding(20.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                                Text("Step ${index + 1} of ${original.steps.size}", style = MaterialTheme.typography.labelLarge)
                                RichMessageText(original.steps[index], style = MaterialTheme.typography.headlineSmall)
                            }
                        }
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                            SigilTextButton({ step-- }, enabled = step > 0) { Glyph("chevron_left", 24); Text("Previous") }
                            SigilTextButton({ step++ }, enabled = step < original.steps.lastIndex) { Text("Next"); Glyph("chevron_right", 24) }
                        }
                    }
                    if (keepAwake != null) Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).toggleable(awake, role = Role.Switch) { awake = it }, verticalAlignment = Alignment.CenterVertically) {
                        Text("Keep screen awake", Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
                        Switch(awake, null)
                    }
                }
            }
        }
    }
}
