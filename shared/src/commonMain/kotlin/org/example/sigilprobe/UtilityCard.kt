package org.sigil

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*
import kotlin.math.floor

val LocalSensitiveCopy = staticCompositionLocalOf<((String) -> Unit)?> { null }

internal fun utilityLabel(value: UtilityContent) = when (value.kind) {
    "calculation" -> "Calculation"; "conversion" -> "Conversion"; "math" -> "Formula"; "qr" -> when (value.qr?.kind) {
        "wifi" -> "Wi-Fi QR code"; "contact" -> "Contact QR code"; else -> "QR code"
    }
    "dice" -> "Dice"; "pick" -> if(value.motion?.kind=="coin")"Coin flip" else "Choice"; "random" -> "Random number"; "swatch" -> "Color"
    "keys" -> "Keyboard shortcut"; "rating" -> "Rating"; "progress" -> "Progress"; "quote" -> "Quote"; else -> "Details"
}
internal fun utilityAction(value: UtilityContent) = when (value.kind) { "qr" -> "QR code"; "math" -> "formula"; else -> utilityLabel(value).lowercase() }

@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
internal fun UtilityCard(value: UtilityContent, mine: Boolean = false, open: (() -> Unit)? = null) {
    if (value.kind == "art") { AsciiArtCard(value.display); return }
    if (value.qr != null) { QrCard(value, mine, open); return }
    if (value.kind == "progress") { ProgressCard(value); return }
    if (value.kind == "rating") { RatingCard(value); return }
    if (value.kind == "random") { NumberPickCard(value); return }
    when (value.kind) { "quote" -> { PullQuoteCard(value); return }; "keys" -> { KeysCard(value); return }; "swatch" -> { SwatchCard(value); return } }
    when (value.kind) { "calculation" -> { CalculationCard(value); return }; "conversion" -> { ConversionCard(value); return }; "math" -> { MathCard(value); return } }
    val clipboard = LocalClipboardManager.current
    val clock=LocalTextMotion.current?.clock
    val motion=LocalMotion.current
    val animate=LocalAppearance.current.messageEffects && !motion.reduced
    val objectMessage=value.motion?.kind in listOf("dice","coin","choice")
    // These carry a figure, not a block: the bubble must shrink to it instead of holding a 200.dp floor.
    val hug = value.kind in listOf("rating","qr")
    fun resultAlpha()=if(animate && value.motion!=null && (clock?.elapsed ?: 12000f)<(clock?.duration(randomizerDuration(value.motion)) ?: randomizerDuration(value.motion)))0f else 1f
    val label = utilityLabel(value)
    val action = utilityAction(value)
    val icon = when (value.kind) {
        "qr" -> "qr_code"
        "dice" -> "casino"; "pick" -> if (value.motion?.kind == "coin") "toll" else "playing_cards"; "random" -> "numbers"; "swatch" -> "palette"
        "keys" -> "keyboard"; "rating" -> "star"; "progress" -> "data_usage"; "quote" -> "format_quote"; else -> "data_object"
    }
    @Composable fun body() {
        if(objectMessage) {
            RandomizerStage(value.motion!!,false,value.rich,if(value.motion.kind=="choice")choiceDescription(value) else null)
            RandomizerCaption(value,resultAlpha()>0f)
            value.secondary?.let {RichMessageText(it,style=MaterialTheme.typography.bodyMedium)}
            return
        }
        value.motion?.let {RandomizerStage(it,false,value.rich)}
        if(value.kind=="dice" && value.details.size>6)Text("6 of ${value.details.size} dice shown",style=MaterialTheme.typography.labelSmall)
        value.rich?.let { RichMessageText(it,Modifier.graphicsLayer {alpha=resultAlpha()},MaterialTheme.typography.bodyLarge) }
        if (value.kind != "rating" && value.display.isNotEmpty() && !(value.kind=="random" && value.motion!=null) && value.motion?.kind!="coin") Text(value.display, modifier=Modifier.graphicsLayer {alpha=resultAlpha()},
            style = if (value.kind == "random") MaterialTheme.typography.headlineMedium.copy(fontFamily = LocalCodeFont.current) else MaterialTheme.typography.bodyLarge,
            maxLines = 3, overflow = TextOverflow.Ellipsis)
        if (value.alternate.isNotEmpty()) Text(value.alternate, style = MaterialTheme.typography.bodyLarge, maxLines = 3, overflow = TextOverflow.Ellipsis)
        if (value.kind !in listOf("rating","progress")) value.ratio?.let { ratio ->
            val ink = LocalContentColor.current
            Canvas(Modifier.fillMaxWidth().height(8.dp).semantics { progressBarRangeInfo = ProgressBarRangeInfo(ratio.coerceIn(0f, 1f), 0f..1f) }) {
                drawRoundRect(ink.copy(alpha = .15f), cornerRadius = androidx.compose.ui.geometry.CornerRadius(size.height / 2))
                drawRoundRect(ink, size = size.copy(width = size.width * ratio.coerceIn(0f, 1f)), cornerRadius = androidx.compose.ui.geometry.CornerRadius(size.height / 2))
            }
        }
        value.secondary?.let { RichMessageText(it, style = MaterialTheme.typography.bodyMedium) }
    }
    Column(Modifier.widthIn(min = if (hug) Dp.Unspecified else 200.dp, max = 280.dp).animateContentSize(motion.tween(MotionMillis)), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if(!objectMessage)Row(if (hug) Modifier else Modifier.fillMaxWidth(), verticalAlignment=Alignment.CenterVertically, horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            Glyph(icon, 20)
            Text(label, if (hug) Modifier else Modifier.weight(1f), style = MaterialTheme.typography.labelMedium)
            value.copy?.let { copy -> SigilIconButton({clipboard.setText(AnnotatedString(copy))}) {Glyph("content_copy",24,"Copy $action")} }
        }
        body()
    }
}
