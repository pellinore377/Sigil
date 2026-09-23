package org.sigil

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.*
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.semantics.*
import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.foundation.text.appendInlineContent
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.Placeholder
import androidx.compose.ui.text.PlaceholderVerticalAlign
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.intl.Locale
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import kotlin.math.roundToInt

private const val DefinitionSenses = 5
private const val MinusSign = "−"

@Composable
internal fun ServiceCard(value: ServiceContent, nowSeconds: Long? = null) = when (value.kind) {
    "translation" -> TranslationCard(value)
    "definition" -> DefinitionCard(value)
    else -> WeatherCard(value, nowSeconds)
}

internal val CardQuiet = .68f
@Composable internal fun quietInk() = LocalContentColor.current.copy(alpha = CardQuiet)

// Small capitals: uppercase, tracked, quiet; screen readers get the words as written.
@Composable internal fun CardCaps(text: String, modifier: Modifier = Modifier, spoken: String = text) {
    Text(text.uppercase(), modifier.clearAndSetSemantics { this.text = AnnotatedString(spoken) }, style = MaterialTheme.typography.labelSmall.copy(letterSpacing = 1.4.sp),
        color = quietInk(), maxLines = 1, overflow = TextOverflow.Ellipsis)
}

// Widens a row on both sides so its press fill runs past the shared text edge.
internal fun Modifier.cardBleed(by: Dp) = layout { measurable, constraints ->
    val extra = by.roundToPx() * 2
    val widened = if (constraints.hasBoundedWidth) constraints.copy(minWidth = constraints.minWidth + extra, maxWidth = constraints.maxWidth + extra) else constraints
    val placeable = measurable.measure(widened)
    layout(maxOf(0, placeable.width - extra).coerceIn(constraints.minWidth, if (constraints.hasBoundedWidth) constraints.maxWidth else Constraints.Infinity), placeable.height) { placeable.place(-extra / 2, 0) }
}

// A full-width row target with the ink press fill on a 14dp squircle.
@Composable internal fun cardPress(role: Role, label: String? = null, action: () -> Unit): Modifier {
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val hovered by interaction.collectIsHoveredAsState()
    val fill = if (pressed || hovered) .07f else 0f
    return Modifier.cardBleed(8.dp).clip(RoundedCornerShape(14.dp)).background(LocalContentColor.current.copy(alpha = fill))
        .clickable(interaction, null, role = role, onClickLabel = label, onClick = action).padding(horizontal = 8.dp)
}

@Composable
private fun ServiceFoot(value: ServiceContent, trailing: @Composable RowScope.() -> Unit = {}) {
    val uri = LocalUriHandler.current
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        val source = value.source
        Row(Modifier.weight(1f).then(if (source != null) cardPress(Role.Button, "Open source") { uri.openUri(source) }.heightIn(min = 48.dp) else Modifier),
            verticalAlignment = Alignment.CenterVertically) {
            CompositionLocalProvider(LocalContentColor provides quietInk()) {
                if (source == null) RichMessageText(value.attribution, Modifier.weight(1f, false), MaterialTheme.typography.labelMedium)
                else Text(buildAnnotatedString { append(value.attribution.text); append("\u00A0"); appendInlineContent("open") }, Modifier.weight(1f, false).clearAndSetSemantics { text = AnnotatedString(value.attribution.text) },
                    style = MaterialTheme.typography.labelMedium, inlineContent = mapOf("open" to InlineTextContent(Placeholder(1.2.em, 1.2.em, PlaceholderVerticalAlign.TextCenter)) { Glyph("open_in_new", 16) }))
            }
        }
        trailing()
    }
}

private val languageNames = mapOf("af" to "Afrikaans", "ar" to "Arabic", "bg" to "Bulgarian", "bn" to "Bengali", "ca" to "Catalan", "cs" to "Czech",
    "cy" to "Welsh", "da" to "Danish", "de" to "German", "el" to "Greek", "en" to "English", "eo" to "Esperanto", "es" to "Spanish", "et" to "Estonian",
    "eu" to "Basque", "fa" to "Persian", "fi" to "Finnish", "fil" to "Filipino", "fr" to "French", "ga" to "Irish", "gl" to "Galician", "gu" to "Gujarati",
    "he" to "Hebrew", "iw" to "Hebrew", "hi" to "Hindi", "hr" to "Croatian", "hu" to "Hungarian", "hy" to "Armenian", "id" to "Indonesian", "is" to "Icelandic",
    "it" to "Italian", "ja" to "Japanese", "ka" to "Georgian", "kk" to "Kazakh", "km" to "Khmer", "kn" to "Kannada", "ko" to "Korean", "la" to "Latin",
    "lt" to "Lithuanian", "lv" to "Latvian", "mk" to "Macedonian", "ml" to "Malayalam", "mr" to "Marathi", "ms" to "Malay", "my" to "Burmese", "nb" to "Norwegian",
    "ne" to "Nepali", "nl" to "Dutch", "no" to "Norwegian", "pa" to "Punjabi", "pl" to "Polish", "pt" to "Portuguese", "ro" to "Romanian", "ru" to "Russian",
    "sk" to "Slovak", "sl" to "Slovenian", "sq" to "Albanian", "sr" to "Serbian", "sv" to "Swedish", "sw" to "Swahili", "ta" to "Tamil", "te" to "Telugu",
    "th" to "Thai", "tl" to "Tagalog", "tr" to "Turkish", "uk" to "Ukrainian", "ur" to "Urdu", "uz" to "Uzbek", "vi" to "Vietnamese", "zh" to "Chinese", "zu" to "Zulu")

internal fun sameLanguage(code: String, viewer: String) = code.trim().split('-', '_')[0].lowercase().let { it == viewer.lowercase() || it == "iw" && viewer == "he" || it == "he" && viewer == "iw" }
// Picker entries: names shown, codes sent.
internal val pickerLanguages = (languageNames.filterKeys { it != "iw" && it != "no" } + ("zh-TW" to "Chinese (Traditional)")).entries.map { it.key to it.value }.sortedBy { it.second }
internal fun languageName(code: String): String {
    val parts = code.trim().split('-', '_')
    val name = languageNames[parts[0].lowercase()] ?: return code.trim().uppercase()
    return if (parts.size > 1) "$name (${parts.drop(1).joinToString("-").uppercase()})" else name
}

internal class TranslationLanguages(val source: String, val target: String, val detected: Boolean)
// Presentation writes "es (detected) → en".
internal fun translationLanguages(value: String): TranslationLanguages? {
    val (from, to) = value.split(" → ").takeIf { it.size == 2 }?.map(String::trim) ?: return null
    val detected = from.endsWith("(detected)")
    val source = from.removeSuffix("(detected)").trim()
    return if (source.isEmpty() || to.isEmpty()) null else TranslationLanguages(languageName(source), languageName(to), detected)
}

@Composable
private fun TranslationCard(value: ServiceContent) {
    val languages = remember(value.language) { translationLanguages(value.language) }
    Column(Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).fillMaxWidth().padding(vertical = 4.dp)) {
        if (languages != null) CardCaps(languages.target, spoken = "Translation into ${languages.target}")
        else CardCaps("Translation")
        Spacer(Modifier.height(4.dp))
        RichMessageText(value.title, style = MaterialTheme.typography.titleMedium)
        value.original?.let { original ->
            Spacer(Modifier.height(12.dp))
            if (languages != null) CardCaps(languages.source + if (languages.detected) " · detected" else "",
                spoken = "Original, ${languages.source}" + if (languages.detected) ", detected" else "")
            else CardCaps("Original")
            Spacer(Modifier.height(4.dp))
            CompositionLocalProvider(LocalContentColor provides quietInk()) { RichMessageText(original, style = MaterialTheme.typography.bodyMedium) }
        }
        Spacer(Modifier.height(8.dp))
        ServiceFoot(value)
    }
}

@Composable
private fun DefinitionCard(value: ServiceContent) {
    val uri = LocalUriHandler.current
    var expanded by remember(value) { mutableStateOf(false) }
    val shown = if (expanded) value.senses else value.senses.take(DefinitionSenses)
    val numbered = value.senses.size > 1
    Column(Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).fillMaxWidth().padding(vertical = 4.dp)
        .animateContentSize(LocalMotion.current.tween(MotionMillis))) {
        RichMessageText(value.title, Modifier.semantics { heading(); contentDescription = "Definition. ${value.title.text}" }, MaterialTheme.typography.headlineMedium)
        val language = value.language.takeIf { it.isNotBlank() && !sameLanguage(it, Locale.current.language) }?.let(::languageName)
        val meta = listOfNotNull(value.pronunciation?.text, language)
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (meta.isNotEmpty()) Text(meta.joinToString(" · "), Modifier.weight(1f, false).semantics {
                contentDescription = listOfNotNull(value.pronunciation?.let { "Pronounced ${it.text}" }, language).joinToString(", ")
            }, style = MaterialTheme.typography.bodyMedium, color = quietInk())
            value.audio?.let { audio -> SigilIconButton({ uri.openUri(audio) }, Modifier.offset(x = if (meta.isEmpty()) (-12).dp else 0.dp)) { Glyph("volume_up", 20, "Play pronunciation") } }
        }
        Spacer(Modifier.height(12.dp))
        if (value.senses.isEmpty()) Text("No definition found.", style = MaterialTheme.typography.bodyMedium, color = quietInk())
        shown.forEachIndexed { index, sense ->
            val heading = index == 0 || sense.part.text != shown[index - 1].part.text
            if (heading && sense.part.text.isNotBlank()) {
                if (index > 0) Spacer(Modifier.height(8.dp))
                CardCaps(sense.part.text)
                Spacer(Modifier.height(4.dp))
            } else if (index > 0) Spacer(Modifier.height(8.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (numbered) Text("${index + 1}.", Modifier.widthIn(min = 20.dp), style = MaterialTheme.typography.bodyMedium.copy(fontFeatureSettings = "tnum, lnum"), color = quietInk())
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    RichMessageText(sense.definition, style = MaterialTheme.typography.bodyMedium)
                    sense.example?.let { CompositionLocalProvider(LocalContentColor provides quietInk()) { RichMessageText(it, style = MaterialTheme.typography.bodyMedium.copy(fontStyle = FontStyle.Italic)) } }
                }
            }
        }
        if (value.senses.size > DefinitionSenses) SigilTextButton({ expanded = !expanded }, Modifier.offset(x = (-12).dp)) {
            Text(if (expanded) "Show fewer" else "Show all ${value.senses.size}")
        }
        Spacer(Modifier.height(8.dp))
        ServiceFoot(value)
    }
}

// "21.0 °C" reads as 21°; weather is whole degrees with a true minus sign.
internal fun degrees(reading: String): String {
    val number = Regex("^-?\\d+(\\.\\d+)?").find(reading.trim())?.value?.toDoubleOrNull() ?: return reading
    val whole = number.roundToInt()
    return (if (whole < 0) MinusSign else "") + "${kotlin.math.abs(whole)}°"
}
// Drops float noise from a leading figure: "8.0 km/h N" becomes "8 km/h N".
internal fun wholeReading(reading: String): String {
    val match = Regex("^-?\\d+(\\.\\d+)?").find(reading.trim()) ?: return reading
    val whole = match.value.toDouble().roundToInt()
    return (if (whole < 0) MinusSign else "") + "${kotlin.math.abs(whole)}" + reading.trim().substring(match.value.length)
}
// Minutes past midnight from "Mon, Sep 15 · 3:00 PM UTC".
internal fun weatherMinutes(date: String): Int? {
    val time = Regex("(\\d{1,2}):(\\d{2})\\s*([AP]M)").find(date) ?: return null
    val (hour, minute, half) = time.destructured
    return (hour.toInt() % 12 + if (half == "PM") 12 else 0) * 60 + minute.toInt()
}
internal fun weatherHour(date: String): String {
    val minutes = weatherMinutes(date) ?: return date
    val hour = minutes / 60 % 12
    return "${if (hour == 0) 12 else hour} ${if (minutes >= 720) "PM" else "AM"}"
}
// The hours from the reading onward, "Now" first.
internal fun upcomingHours(current: WeatherConditions, hours: List<WeatherConditions>): List<WeatherConditions> {
    val now = current.key to (weatherMinutes(current.date) ?: 0)
    return hours.filter { hour -> val m = weatherMinutes(hour.date) ?: return@filter false; hour.key > now.first || hour.key == now.first && m / 60 > now.second / 60 }
}
internal fun imperialRegion() = Locale.current.region.uppercase() in setOf("US", "LR", "MM", "BS", "BZ", "KY", "PW")

// One unit choice for every weather card and quote; the app binds it to settings.
internal object WeatherUnits {
    var imperial by mutableStateOf(imperialRegion())
    private var save: (Boolean) -> Unit = {}
    fun bind(saved: String?, store: (Boolean) -> Unit) { saved?.let { imperial = it == "imperial" }; save = store }
    fun toggle() { imperial = !imperial; save(imperial) }
}

internal class WeatherTime(val label: String, val now: Boolean, val today: Boolean)
// Snapshot age decides "Now" and "Today"; past the reading's local day the full date shows.
internal fun weatherTime(value: ServiceContent, nowSeconds: Long): WeatherTime {
    val current = value.current
    if (current == null || current.at <= 0) {
        val fresh = !value.historical
        val clock = current?.date?.substringAfter(" · ", "").orEmpty()
        return WeatherTime("As of " + if (fresh && clock.isNotEmpty()) clock else current?.date ?: value.stamp, fresh && current != null, fresh)
    }
    val age = nowSeconds - current.at
    val dayLeft = (24 * 60 - (weatherMinutes(current.date) ?: 0)) * 60L
    val today = age in -300 until dayLeft
    return WeatherTime("As of " + if (today) current.date.substringAfter(" · ") else current.date, age in -300 until 3600, today)
}

@Composable
private fun UnitSwitch(imperial: Boolean, modifier: Modifier = Modifier) {
    val ink = LocalContentColor.current
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val hovered by interaction.collectIsHoveredAsState()
    Box(modifier.heightIn(min = 48.dp).widthIn(min = 48.dp).clip(RoundedCornerShape(14.dp)).background(ink.copy(alpha = if (pressed || hovered) .07f else 0f))
        .clickable(interaction, null, role = Role.Button, onClickLabel = if (imperial) "Show Celsius" else "Show Fahrenheit") { WeatherUnits.toggle() }
        .semantics { contentDescription = if (imperial) "Show Celsius and kilometres" else "Show Fahrenheit and miles"; stateDescription = if (imperial) "Fahrenheit" else "Celsius" }
        .padding(horizontal = 8.dp), contentAlignment = Alignment.Center) {
        val quiet = ink.copy(alpha = CardQuiet)
        Text(buildAnnotatedString {
            withStyle(SpanStyle(color = if (imperial) ink else quiet)) { append("°F") }
            withStyle(SpanStyle(color = quiet)) { append(" / ") }
            withStyle(SpanStyle(color = if (imperial) quiet else ink)) { append("°C") }
        }, style = MaterialTheme.typography.labelMedium, maxLines = 1)
    }
}

@OptIn(ExperimentalLayoutApi::class, kotlin.time.ExperimentalTime::class)
@Composable
private fun WeatherCard(value: ServiceContent, nowSeconds: Long? = null) {
    val imperial = WeatherUnits.imperial
    val unit = if (imperial) 1 else 0
    val ink = LocalContentColor.current
    val quiet = quietInk()
    val numerals = MaterialTheme.typography.bodyMedium.copy(fontFeatureSettings = "tnum, lnum")
    val clock = nowSeconds ?: remember(value) { kotlin.time.Clock.System.now().epochSeconds }
    val time = remember(value, clock) { weatherTime(value, clock) }
    val today = value.days.firstOrNull { it.key == value.today }
    Column(Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).fillMaxWidth().padding(vertical = 4.dp)
        .animateContentSize(LocalMotion.current.tween(MotionMillis))) {
        RichMessageText(value.title, Modifier.semantics { heading(); contentDescription = "Weather. ${value.title.text}" }, MaterialTheme.typography.titleMedium)
        Text(time.label, Modifier.padding(top = 4.dp), style = MaterialTheme.typography.labelMedium, color = quiet, maxLines = 2)
        val current = value.current
        if (current != null) {
            Spacer(Modifier.height(12.dp))
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                Text(degrees(current.temperature[unit]), Modifier.semantics { contentDescription = current.temperature[unit] },
                    style = MaterialTheme.typography.displayMedium.copy(fontFeatureSettings = "tnum, lnum"), maxLines = 1)
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Glyph(current.icon, 20)
                        RichMessageText(current.description, Modifier.weight(1f, false), MaterialTheme.typography.bodyMedium)
                    }
                    val range = listOfNotNull(today?.let { "H ${degrees(it.high[unit])} · L ${degrees(it.low[unit])}" }, current.feelsLike?.let { "Feels ${degrees(it[unit])}" })
                    if (range.isNotEmpty()) Text(range.joinToString(" · "), Modifier.semantics {
                        contentDescription = listOfNotNull(today?.let { "High ${it.high[unit]}, low ${it.low[unit]}" }, current.feelsLike?.let { "feels like ${it[unit]}" }).joinToString(", ")
                    }, style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum, lnum"), color = quiet)
                }
                UnitSwitch(imperial, Modifier.offset(x = 8.dp))
            }
            val wind = wholeReading(current.wind[unit]).split(' ')
            val direction = wind.lastOrNull()?.takeIf { wind.size > 2 && it.all(Char::isLetter) }
            val metrics = listOfNotNull(current.chance?.let { "Rain" to it }, (if (direction != null) "Wind $direction" else "Wind") to (if (direction != null) wind.dropLast(1) else wind).joinToString(" "),
                current.humidity?.let { "Humidity" to it }, current.uv?.let { "UV" to wholeReading(it) }, current.rain?.takeIf { current.chance == null }?.let { "Rainfall" to it })
            val across = if (metrics.size == 4) 4 else 3
            Spacer(Modifier.height(16.dp))
            metrics.chunked(across).forEachIndexed { row, items ->
                if (row > 0) Spacer(Modifier.height(12.dp))
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    items.forEach { (name, reading) ->
                        Column(Modifier.weight(1f).semantics(mergeDescendants = true) {}) {
                            Text(name, style = MaterialTheme.typography.labelMedium, color = quiet, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(reading, style = numerals, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                    repeat(across - items.size) { Spacer(Modifier.weight(1f)) }
                }
            }
            val hours = remember(value) { upcomingHours(current, value.hours) }
            if (hours.isNotEmpty()) BoxWithConstraints(Modifier.fillMaxWidth().padding(top = 16.dp)) {
                val columns = (maxWidth / 52.dp).toInt().coerceIn(3, 6)
                // A stale reading is already the hero; the strip then starts at the next hour.
                val shown = if (time.now) listOf(current) + hours.take(columns - 1) else hours.take(columns)
                Row(Modifier.fillMaxWidth().semantics(mergeDescendants = true) {
                    contentDescription = "Next hours: " + hours.take(columns - if (time.now) 1 else 0).joinToString("; ") { "${weatherHour(it.date)} ${it.temperature[unit]}" }
                }) {
                    shown.forEachIndexed { index, hour ->
                        Column(Modifier.weight(1f).clearAndSetSemantics {}, horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text(if (index == 0 && time.now) "Now" else weatherHour(hour.date), style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum, lnum"), color = quiet, maxLines = 1)
                            Glyph(hour.icon, 20)
                            Text(degrees(hour.temperature[unit]), style = numerals, maxLines = 1)
                        }
                    }
                }
            }
        }
        if (value.days.size > 1) {
            Spacer(Modifier.height(16.dp))
            val floor = value.days.minOf { it.low[unit].leadingNumber() }
            val ceiling = value.days.maxOf { it.high[unit].leadingNumber() }
            value.days.forEach { day ->
                val low = day.low[unit].leadingNumber(); val high = day.high[unit].leadingNumber()
                val name = if (day.key == value.today && time.today) "Today" else day.date.substringBefore(',')
                Row(Modifier.fillMaxWidth().heightIn(min = 36.dp).clearAndSetSemantics {
                    contentDescription = "$name: ${day.description.text}, high ${day.high[unit]}, low ${day.low[unit]}, ${day.chance} chance of rain"
                }, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(name, Modifier.width(56.dp), style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Glyph(day.icon, 20)
                    Text(day.chance.takeIf { (it.leadingNumber()) >= 10.0 }.orEmpty(), Modifier.width(36.dp), style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum, lnum"), color = quiet, maxLines = 1)
                    Text(degrees(day.low[unit]), Modifier.width(36.dp), style = numerals, color = quiet, textAlign = TextAlign.End, maxLines = 1)
                    Canvas(Modifier.weight(1f).height(4.dp)) {
                        val span = (ceiling - floor).takeIf { it > 0 } ?: 1.0
                        val round = androidx.compose.ui.geometry.CornerRadius(size.height / 2)
                        drawRoundRect(ink.copy(alpha = .12f), cornerRadius = round)
                        val start = ((low - floor) / span).toFloat() * size.width
                        val end = maxOf(start + size.height, ((high - floor) / span).toFloat() * size.width)
                        val left = if (layoutDirection == LayoutDirection.Rtl) size.width - end else start
                        drawRoundRect(ink, androidx.compose.ui.geometry.Offset(left, 0f), size.copy(width = end - start), round)
                    }
                    Text(degrees(day.high[unit]), Modifier.width(36.dp), style = numerals, textAlign = TextAlign.End, maxLines = 1)
                }
            }
            if (current == null) Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) { UnitSwitch(imperial, Modifier.offset(x = 8.dp)) }
        }
        Spacer(Modifier.height(8.dp))
        ServiceFoot(value)
    }
}

private fun RichText.plainUnlessHidden() = text.takeUnless { spans.any { it.reveal.isNotEmpty() || it.redaction > 0 } }
// What the message menu's Copy yields for a service card: the result, never the lookup syntax.
internal fun serviceCopy(value: ServiceContent, imperial: Boolean = WeatherUnits.imperial): String? = when (value.kind) {
    "translation" -> value.copy
    "definition" -> value.title.plainUnlessHidden()?.let { word ->
        val senses = value.senses.mapNotNull { sense -> sense.copy?.let { listOfNotNull(sense.part.text.takeIf(String::isNotBlank), it).joinToString(". ") } }
        (listOf(word) + senses.mapIndexed { i, line -> if (senses.size > 1) "${i + 1}. $line" else line }).joinToString("\n")
    }
    else -> value.title.plainUnlessHidden()?.let { place ->
        val current = value.current ?: return@let null
        "$place · ${degrees(current.temperature[if (imperial) 1 else 0])}, ${current.description.text} · ${current.date}"
    }
}
internal fun serviceMessageCopy(message: ChatMessage) = message.parts.firstNotNullOfOrNull { it.service?.let(::serviceCopy) }
private fun String.leadingNumber() = Regex("^-?\\d+(\\.\\d+)?").find(trim())?.value?.toDoubleOrNull() ?: 0.0
