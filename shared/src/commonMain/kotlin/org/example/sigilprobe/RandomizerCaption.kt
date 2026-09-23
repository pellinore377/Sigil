package org.sigil

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.layout.*
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal data class DiceGroup(val sides: Int, val faces: List<Int>)

// Consecutive "d6 · 3" details of one size form a group, as they were written.
internal fun diceGroups(details: List<RichText>): List<DiceGroup> {
    val rolls = details.mapNotNull { d -> Regex("""d(\d+) · (\d+)""").matchEntire(d.text.trim())?.destructured?.let { (s, f) -> s.toIntOrNull()?.let { a -> f.toIntOrNull()?.let { b -> a to b } } } }
    return rolls.fold(mutableListOf<DiceGroup>()) { groups, (sides, face) ->
        if (groups.lastOrNull()?.sides == sides) groups[groups.lastIndex] = groups.last().copy(faces = groups.last().faces + face)
        else groups += DiceGroup(sides, listOf(face))
        groups
    }
}

private fun groupedTotal(raw: String) = raw.toLongOrNull()?.toString()?.reversed()?.chunked(3)?.joinToString(",")?.reversed() ?: raw

internal fun diceGroupLine(group: DiceGroup): Pair<String, String> {
    val label = "${if (group.faces.size > 1) group.faces.size else ""}d${group.sides}"
    val sum = group.faces.sum()
    val value = when {
        group.faces.size == 1 -> "$sum"
        group.faces.size > 6 -> "= $sum"
        else -> group.faces.joinToString(" · ") + " = $sum"
    }
    return label to value
}

internal data class CaptionModel(val label: String, val figure: String, val breakdown: Boolean, val note: String?, val spoken: String)

private fun spokenFaces(faces: List<Int>) = if (faces.size <= 1) faces.joinToString() else faces.dropLast(1).joinToString(", ") + " and " + faces.last()

// What the caption shows and says; the stage above it carries only the type word.
internal fun captionModel(coin: Boolean, result: String, groups: List<DiceGroup>, shownDice: Int): CaptionModel {
    val count = groups.sumOf { it.faces.size }
    val figure = if (coin) result else groupedTotal(result)
    val label = if (coin || count <= 1) "Result" else "Total"
    // A single large group would only repeat the total.
    val breakdown = !coin && count > 1 && (groups.size > 1 || count <= 6)
    val note = if (!coin && shownDice < count && count > 6) "$shownDice of $count dice shown" else null
    val parts = if (coin || count <= 1) "" else ": " + groups.joinToString("; ") { g ->
        val name = diceGroupLine(g).first
        if (g.faces.size > 6) "$name, ${g.faces.sum()}" else "$name, ${spokenFaces(g.faces)}"
    }
    val spoken = if (coin) "Coin flip. Result: $figure" else if (count <= 1) "Dice roll. Result: $figure" else "Dice roll. Total $figure$parts"
    return CaptionModel(label, figure, breakdown, note, spoken)
}

/** Result line under dice and coin objects: a small-caps label and the figure, then the dice breakdown. */
@OptIn(ExperimentalLayoutApi::class)
@Composable internal fun RandomizerCaption(value: UtilityContent, shown: Boolean) {
    val motion = value.motion ?: return
    val coin = motion.kind == "coin"
    if (!coin && motion.kind != "dice" || motion.result.isEmpty()) return
    val groups = if (coin) emptyList() else diceGroups(value.details)
    val model = captionModel(coin, motion.result, groups, motion.dice.size)
    val ink = LocalContentColor.current
    val quiet = ink.copy(alpha = .68f)
    val alpha by animateFloatAsState(if (shown) 1f else 0f, LocalMotion.current.tween(MotionMillis), label = "Randomizer result")
    val end = LocalMaterialOutgoing.current
    val mono = MaterialTheme.typography.labelMedium.copy(fontFamily = LocalCodeFont.current, fontFeatureSettings = "tnum, lnum")
    Column(Modifier.fillMaxWidth().padding(top = 4.dp).graphicsLayer { this.alpha = alpha }
        .clearAndSetSemantics { if (shown) contentDescription = model.spoken },
        horizontalAlignment = if (end) Alignment.End else Alignment.Start, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(model.label.uppercase(), Modifier.alignByBaseline(), style = MaterialTheme.typography.labelSmall.copy(letterSpacing = 1.4.sp), color = quiet)
            Text(model.figure, Modifier.alignByBaseline(), style = MaterialTheme.typography.headlineSmall.copy(fontFeatureSettings = "tnum, lnum"), maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        // One quiet mono run per group; the dice label sits a step quieter than its values.
        if (model.breakdown) FlowRow(horizontalArrangement = Arrangement.spacedBy(12.dp, if (end) Alignment.End else Alignment.Start), verticalArrangement = Arrangement.spacedBy(0.dp)) {
            groups.take(6).forEach { group ->
                val (name, sum) = diceGroupLine(group)
                Text(buildAnnotatedString { withStyle(SpanStyle(color = ink.copy(alpha = .6f))) { append(name) }; append(" "); append(sum) }, style = mono, color = quiet, maxLines = 1)
            }
            if (groups.size > 6) Text("+${groups.size - 6} more", style = mono, color = quiet)
        }
        model.note?.let { Text(it, style = MaterialTheme.typography.labelMedium, color = quiet) }
    }
}
