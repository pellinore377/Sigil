package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.takeOrElse
import androidx.compose.ui.text.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

private const val CodeLineCap = 24

internal fun visibleCodeBlocks(value: RichText) = value.blocks.filter { block ->
    block.kind == "code" && block.start >= 0 && block.end <= value.text.length && block.start < block.end &&
        value.spans.none { it.reveal.isNotEmpty() && it.start < block.end && it.end > block.start }
}
internal fun richSlice(value: RichText, start: Int, end: Int) = RichText(value.text.substring(start, end),
    value.spans.filter { it.start < end && it.end > start }.map { it.copy(start = maxOf(it.start, start) - start, end = minOf(it.end, end) - start) },
    value.blocks.filter { it.start < end && it.end > start }.map { it.copy(start = maxOf(it.start, start) - start, end = minOf(it.end, end) - start) },
    value.codeTokens.filter { it.start >= start && it.end <= end }.map { it.copy(start = it.start - start, end = it.end - start) },
    value.motion.mapNotNull { run ->run.copy(units=run.units.filter {it.first>=start && it.second<=end}.map {it.first-start to it.second-start}).takeIf {it.units.isNotEmpty()} })

@Composable
internal fun CodeBlock(value: RichText, language: String) = CodePanel(value, language, false)

@Composable
internal fun AsciiArt(value: RichText) = CodePanel(value, "", true)

// An opaque panel, not an alpha wash: the block must read identically on a primary bubble and on a surfaceVariant one.
@Composable
private fun CodePanel(value: RichText, language: String, hug: Boolean) {
    val scheme = MaterialTheme.colorScheme
    val panel = lerp(scheme.background, scheme.onBackground, if (scheme.background.luminance() < .18f) .07f else .90f)
    val ink = listOf(scheme.onBackground, scheme.background).maxByOrNull { contrastWith(it, panel) } ?: scheme.onBackground
    val lines = remember(value.text) { value.text.count { it == '\n' } + if (value.text.endsWith('\n')) 0 else 1 }
    val hidden = lines - CodeLineCap
    val caption = language.takeIf { it.isNotEmpty() }?.replaceFirstChar { it.uppercase() }
    Column(Modifier.then(if (hug) Modifier.widthIn(max = 280.dp) else Modifier.fillMaxWidth()).clip(RoundedCornerShape(12.dp)).background(panel)
        .padding(horizontal = 12.dp, vertical = if (hug) 8.dp else 10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        CompositionLocalProvider(LocalMessageSurface provides panel, LocalContentColor provides ink) {
            Box(Modifier.then(if (hug) Modifier else Modifier.fillMaxWidth()).horizontalScroll(rememberScrollState())) { CodeText(value, false, CodeLineCap) }
            if (caption != null || hidden > 0) Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                if (hidden > 0) Text("+$hidden more ${if (hidden == 1) "line" else "lines"}", style = MaterialTheme.typography.labelSmall, color = ink.copy(alpha = .55f), maxLines = 1)
                Spacer(Modifier.weight(1f))
                if (caption != null) Text(caption, style = MaterialTheme.typography.labelSmall, color = ink.copy(alpha = .55f), maxLines = 1)
            }
        }
    }
}

private fun contrastWith(a: Color, b: Color): Float {
    val x = a.luminance(); val y = b.luminance()
    return (maxOf(x, y) + .05f) / (minOf(x, y) + .05f)
}

@Composable
private fun CodeText(value: RichText, wrap: Boolean, maxLines: Int) {
    val background = LocalMessageSurface.current.takeOrElse { MaterialTheme.colorScheme.surface }
    val foreground = LocalContentColor.current
    val text = remember(value, background, foreground) {
        buildAnnotatedString {
            append(value.text)
            value.codeTokens.forEach { token ->
                if (token.start >= 0 && token.end <= length && token.start < token.end) {
                    val color = when (token.role) { "keyword" -> "purple2"; "number" -> "blue2"; "string" -> "green2"; "comment" -> "gray2"; else -> null }
                    if (color != null) addStyle(SpanStyle(color = textColor(color, background)), token.start, token.end)
                }
            }
        }
    }
    Text(text, style = MaterialTheme.typography.bodyMedium.copy(fontFamily = LocalCodeFont.current), color = foreground, softWrap = wrap, maxLines = maxLines, overflow = TextOverflow.Clip)
}
