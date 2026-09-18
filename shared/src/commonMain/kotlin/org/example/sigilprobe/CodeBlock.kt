package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.takeOrElse
import androidx.compose.ui.text.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

// Panel, ink and token hues read from the reference screenshot.
private val CodePanelDark = Color(0xff242428)
private val CodeInkDark = Color(0xffbec3cc)

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
internal fun CodeBlock(value: RichText, language: String, modifier: Modifier = Modifier, shape: Shape = RoundedCornerShape(12.dp)) = CodePanel(value, language, false, modifier, shape)

@Composable
internal fun AsciiArt(value: RichText) = CodePanel(value, "", true, Modifier, RoundedCornerShape(12.dp))

// An opaque panel, not an alpha wash: the block must read identically on a primary bubble and on a surfaceVariant one.
@Composable
private fun CodePanel(value: RichText, language: String, hug: Boolean, modifier: Modifier, shape: Shape) {
    val scheme = MaterialTheme.colorScheme
    val dark = scheme.background.luminance() < .18f
    val panel = if (dark) CodePanelDark else lerp(scheme.background, scheme.onBackground, .055f)
    val ink = if (dark) CodeInkDark else scheme.onBackground
    val label = language.takeIf { it.isNotEmpty() }?.lowercase()
    Box(modifier.then(if (hug) Modifier else Modifier.fillMaxWidth()).clip(shape).background(panel)
        .padding(start = 14.dp, end = 14.dp, top = 12.dp, bottom = if (label == null) 12.dp else 10.dp)) {
        CompositionLocalProvider(LocalMessageSurface provides panel, LocalContentColor provides ink) {
            Column(Modifier.then(if (hug) Modifier else Modifier.fillMaxWidth())) {
                CodeText(value, true, Int.MAX_VALUE)
                if (label != null) Box(Modifier.fillMaxWidth().padding(top = 8.dp)) {
                    Text(label, Modifier.align(Alignment.CenterEnd).clip(RoundedCornerShape(5.dp))
                        .background(lerp(panel, ink, .10f)).padding(horizontal = 6.dp, vertical = 2.dp),
                        style = MaterialTheme.typography.labelSmall, color = ink.copy(alpha = .55f), maxLines = 1)
                }
            }
        }
    }
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
                    codeInk(token.role, background)?.let { addStyle(SpanStyle(color = it), token.start, token.end) }
                }
            }
        }
    }
    Text(text, style = MaterialTheme.typography.bodyMedium.copy(fontFamily = LocalCodeFont.current), color = foreground, softWrap = wrap, maxLines = maxLines, overflow = TextOverflow.Clip)
}

/** Token hues sampled from the reference screenshot, with themed equivalents on a light panel. */
private fun codeInk(role: String, background: Color): Color? {
    val dark = background.luminance() < .3f
    return when (role) {
        "keyword" -> if (dark) Color(0xffbf95b7) else textColor("purple2", background)
        "function" -> if (dark) Color(0xff93aec4) else textColor("blue2", background)
        "string" -> if (dark) Color(0xffa8c48f) else textColor("green2", background)
        "number" -> if (dark) Color(0xffd08a71) else textColor("orange2", background)
        "comment" -> if (dark) Color(0xff6c7177) else textColor("gray2", background)
        else -> null
    }
}
