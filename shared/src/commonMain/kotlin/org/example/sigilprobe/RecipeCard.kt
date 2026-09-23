package org.sigil

import androidx.compose.animation.*
import androidx.compose.foundation.*
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.*
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.FirstBaseline
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*
import kotlinx.coroutines.CancellationException
import kotlin.math.abs

private const val RecipeIngredients = 5
private const val RecipeSteps = 3

// Checks and servings outlive the card's composition, so the card and the cooking view agree for the session.
internal class RecipeSession { var checked by mutableStateOf(emptySet<Int>()); var serves by mutableStateOf<Int?>(null) }
private const val RecipeSessionLimit = 20
private val recipeSessions = LinkedHashMap<String, RecipeSession>()
// Most recently opened last; the oldest drop past the limit.
internal fun recipeSession(key: String): Pair<RecipeSession, Boolean> {
    val found = recipeSessions.remove(key)
    val session = found ?: RecipeSession()
    recipeSessions[key] = session
    while (recipeSessions.size > RecipeSessionLimit) recipeSessions.remove(recipeSessions.keys.first())
    return session to (found == null)
}
internal fun RecipeSession.encode() = checked.sorted().joinToString(",") + "|" + (serves ?: "")
internal fun RecipeSession.restore(saved: String) {
    if (saved.isEmpty()) return
    checked = saved.substringBefore('|').split(',').mapNotNull(String::toIntOrNull).toSet()
    serves = saved.substringAfter('|', "").toIntOrNull()
}
// Saved state brings checks and servings back after the process is reclaimed mid-recipe.
@Composable internal fun rememberRecipeSession(message: ChatMessage, part: MessagePart): RecipeSession {
    val key = "${message.author}/${message.id}/${part.id}"
    var saved by rememberSaveable(key) { mutableStateOf("") }
    val session = remember(key) { recipeSession(key).let { (session, fresh) -> if (fresh) session.restore(saved); session } }
    LaunchedEffect(session) { snapshotFlow { session.encode() }.collect { saved = it } }
    return session
}
internal fun recipeAsWritten(text: RichText, scaled: Boolean) = !scaled && text.text.any { it.isDigit() || it in "¼½¾⅓⅔⅛" }

internal class RecipeScaling(val value: RecipeContent, val loading: Boolean, val issue: Boolean, val change: ((Int) -> Unit)?)
@Composable internal fun rememberRecipeScaling(message: ChatMessage, original: RecipeContent, part: MessagePart, session: RecipeSession): RecipeScaling {
    val scale = LocalRecipeScale.current
    var value by remember(original) { mutableStateOf(original) }
    var issue by remember(original) { mutableStateOf(false) }
    var loading by remember(original) { mutableStateOf(false) }
    var attempt by remember(original) { mutableIntStateOf(0) }
    LaunchedEffect(original, session.serves, attempt) {
        val target = session.serves
        if (target == null || target == original.serves) { value = original; issue = false; loading = false; return@LaunchedEffect }
        if (scale == null) return@LaunchedEffect
        loading = true; issue = false
        try { value = scale(message, part, target) }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { issue = true }
        finally { loading = false }
    }
    return RecipeScaling(value, loading, issue, if (scale != null && original.serves != null) { serves -> session.serves = serves.coerceIn(1, 65535); attempt++ } else null)
}

internal fun recipeTime(seconds: Long) = when {
    seconds < 60 -> "$seconds sec"
    seconds < 3600 -> if (seconds % 60 == 0L) "${seconds / 60} min" else "${seconds / 60} min ${seconds % 60} sec"
    else -> "${seconds / 3600} hr" + if (seconds % 3600 >= 60) " ${seconds % 3600 / 60} min" else ""
}
internal fun recipeServings(serves: Int) = if (serves == 1) "1 serving" else "$serves servings"

@Composable
internal fun RecipeCard(message: ChatMessage, part: MessagePart) {
    val original = part.recipe ?: return
    val session = rememberRecipeSession(message, part)
    val scaling = rememberRecipeScaling(message, original, part, session)
    val value = scaling.value
    var expanded by remember(original) { mutableStateOf(false) }
    val quiet = quietInk()
    val more = value.ingredients.size > RecipeIngredients || value.steps.size > RecipeSteps
    Column(Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).fillMaxWidth().padding(vertical = 4.dp)
        .animateContentSize(LocalMotion.current.tween(MotionMillis))) {
        RichMessageText(original.title, Modifier.semantics { heading(); contentDescription = "Recipe. ${original.title.text}" }, MaterialTheme.typography.titleMedium)
        val serves = value.serves
        val time = original.seconds?.let(::recipeTime)
        val change = scaling.change
        // The count sits between its − and + so the stepper reads as one control.
        if (serves != null && change != null) Row(verticalAlignment = Alignment.CenterVertically) {
            Text(time.orEmpty(), Modifier.weight(1f), style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum, lnum"), color = quiet)
            SigilIconButton({ change(serves - 1) }, enabled = !scaling.loading && serves > 1) { Glyph("remove", 20, "Fewer servings") }
            Text(recipeServings(serves), Modifier.semantics { liveRegion = LiveRegionMode.Polite }, style = MaterialTheme.typography.bodyMedium.copy(fontFeatureSettings = "tnum, lnum"))
            SigilIconButton({ change(serves + 1) }, Modifier.offset(x = 12.dp), enabled = !scaling.loading && serves < 65535) { Glyph("add", 20, "More servings") }
        } else if (serves != null || time != null) Text(listOfNotNull(serves?.let(::recipeServings), time).joinToString(" · "), Modifier.padding(top = 4.dp),
            style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum, lnum"), color = quiet)
        if (scaling.issue) Text("Couldn't adjust servings. The written amounts are shown.", style = MaterialTheme.typography.labelMedium, color = quiet)
        else if (serves != null && serves != original.serves && value.scaled.any { it }) Text("Amounts adjusted for ${recipeServings(serves)}", style = MaterialTheme.typography.labelMedium, color = quiet)
        if (value.ingredients.isNotEmpty()) {
            Spacer(Modifier.height(12.dp))
            CardCaps("Ingredients")
            Spacer(Modifier.height(4.dp))
            (if (expanded) value.ingredients else value.ingredients.take(RecipeIngredients)).forEachIndexed { index, text ->
                RecipeIngredient(text, index in session.checked, value.serves != original.serves && recipeAsWritten(text, value.scaled.getOrNull(index) == true)) {
                    session.checked = if (index in session.checked) session.checked - index else session.checked + index
                }
            }
        }
        if (value.steps.isNotEmpty()) {
            Spacer(Modifier.height(12.dp))
            CardCaps("Steps")
            Spacer(Modifier.height(8.dp))
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                (if (expanded) value.steps else value.steps.take(RecipeSteps)).forEachIndexed { index, step ->
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Text("${index + 1}", Modifier.width(24.dp).alignBy(FirstBaseline), style = MaterialTheme.typography.bodyMedium.copy(fontFeatureSettings = "tnum, lnum"), color = quiet, textAlign = TextAlign.Center)
                        RichMessageText(step, Modifier.weight(1f).alignBy(FirstBaseline), MaterialTheme.typography.bodyMedium)
                    }
                }
            }
        }
        if (more) SigilTextButton({ expanded = !expanded }, Modifier.offset(x = (-12).dp).padding(top = 4.dp)) {
            Text(if (expanded) "Show less" else listOfNotNull(
                (value.ingredients.size - RecipeIngredients).takeIf { it > 0 }?.let { "$it more ${if (it == 1) "ingredient" else "ingredients"}" },
                (value.steps.size - RecipeSteps).takeIf { it > 0 }?.let { "$it more ${if (it == 1) "step" else "steps"}" }).joinToString(" and ", "Show "))
        }
    }
}

@Composable
private fun RecipeIngredient(text: RichText, done: Boolean, asWritten: Boolean, toggle: () -> Unit) {
    val ink = LocalContentColor.current
    val motion = LocalMotion.current
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val hovered by interaction.collectIsHoveredAsState()
    Row(Modifier.cardBleed(8.dp).fillMaxWidth().heightIn(min = 48.dp).clip(RoundedCornerShape(14.dp)).background(ink.copy(alpha = if (pressed || hovered) .07f else 0f))
        .toggleable(done, interaction, null, role = Role.Checkbox) { toggle() }.padding(horizontal = 8.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        AnimatedContent(done, transitionSpec = { (fadeIn(motion.enter(MotionInline)) + scaleIn(motion.enter(MotionInline), initialScale = .8f)) togetherWith fadeOut(motion.exit(MotionExit)) }, label = "Ingredient check") { checked ->
            CompositionLocalProvider(LocalContentColor provides if (checked) ink else ink.copy(alpha = CardQuiet)) { Glyph(if (checked) "check_box" else "check_box_outline_blank", 24, filled = checked) }
        }
        Column(Modifier.weight(1f)) {
            CompositionLocalProvider(LocalContentColor provides if (done) ink.copy(alpha = CardQuiet) else ink) {
                RichMessageText(text, style = MaterialTheme.typography.bodyMedium.copy(textDecoration = if (done) TextDecoration.LineThrough else null))
            }
            if (asWritten) Text("Not adjusted", style = MaterialTheme.typography.labelMedium, color = ink.copy(alpha = CardQuiet))
        }
    }
}

@Composable
private fun RecipeMetadata(value: RecipeContent) {
    val pieces = listOfNotNull(value.serves?.let(::recipeServings), value.seconds?.let(::recipeTime))
    if (pieces.isNotEmpty()) Text(pieces.joinToString(" · "), style = MaterialTheme.typography.labelMedium)
}

@Composable
internal fun RecipeDetails(message: ChatMessage, part: MessagePart, dismiss: () -> Unit) {
    val original = part.recipe ?: return
    val session = rememberRecipeSession(message, part)
    val scaling = rememberRecipeScaling(message, original, part, session)
    val value = scaling.value
    val loading = scaling.loading
    val issue = scaling.issue
    var step by remember(original) { mutableIntStateOf(0) }
    var ingredients by remember(original) { mutableStateOf(true) }
    var awake by remember { mutableStateOf(false) }
    val keepAwake = LocalKeepScreenAwake.current
    val motion = LocalMotion.current
    Dialog({ awake = false; dismiss() }, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface) {
                keepAwake?.invoke(awake)
                Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        SigilIconButton({ awake = false; dismiss() }) { Glyph("close", 24, "Close recipe") }
                        Text("Recipe", Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
                    }
                    RichMessageText(original.title, Modifier.heightIn(max = 120.dp).verticalScroll(rememberScrollState()), MaterialTheme.typography.titleMedium)
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Column(Modifier.weight(1f)) { RecipeMetadata(value) }
                        val change = scaling.change
                        val serves = value.serves
                        if (change != null && serves != null) {
                            SigilIconButton({ change(serves - 1) }, enabled = !loading && serves > 1) { Glyph("remove", 20, "Fewer servings") }
                            SigilIconButton({ change(serves + 1) }, enabled = !loading && serves < 65535) { Glyph("add", 20, "More servings") }
                        }
                    }
                    if (loading) Text("Adjusting servings…", style = MaterialTheme.typography.labelMedium)
                    if (issue) Text("Couldn't adjust servings. The previous amounts are still shown.", style = MaterialTheme.typography.bodySmall)
                    if (!issue && value.serves != null && value.serves != original.serves && value.scaled.any { it }) Text("Amounts adjusted for ${recipeServings(value.serves)}", style = MaterialTheme.typography.bodySmall)
                    AppearanceChoices("Cooking view", listOf("Ingredients" to "grocery", "Steps" to "format_list_numbered"), if (ingredients) "Ingredients" else "Steps") { ingredients = it == "Ingredients" }
                    if (ingredients) LazyColumn(Modifier.weight(1f).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        itemsIndexed(value.ingredients, key = { index, _ -> index }) { index, text ->
                            val done = index in session.checked
                            Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(MaterialTheme.shapes.medium).background(MaterialTheme.colorScheme.surfaceVariant)
                                .toggleable(done, role = Role.Checkbox) { session.checked = if (done) session.checked - index else session.checked + index }.padding(12.dp),
                                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                Glyph(if (done) "check_box" else "check_box_outline_blank", 24)
                                Column(Modifier.weight(1f).alpha(if (done) .6f else 1f)) {
                                    RichMessageText(text)
                                    if (value.serves != original.serves && recipeAsWritten(text, value.scaled.getOrNull(index) == true)) Text("Not adjusted", style = MaterialTheme.typography.labelMedium)
                                }
                            }
                        }
                    } else {
                        AnimatedContent(step, Modifier.weight(1f).fillMaxWidth(), transitionSpec = {
                            (slideInHorizontally(motion.enter(MotionInline)) { if (targetState > initialState) it else -it } + fadeIn(motion.enter(MotionInline))) togetherWith
                                (slideOutHorizontally(motion.exit(MotionQuick)) { if (targetState > initialState) -it else it } + fadeOut(motion.exit(MotionExit)))
                        }, label = "Recipe step") { index ->
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
                            SigilTextButton({ step-- }, enabled = step > 0) { Glyph("chevron_left", 20); Spacer(Modifier.width(8.dp)); Text("Previous") }
                            SigilTextButton({ step++ }, enabled = step < original.steps.lastIndex) { Text("Next"); Spacer(Modifier.width(8.dp)); Glyph("chevron_right", 20) }
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
