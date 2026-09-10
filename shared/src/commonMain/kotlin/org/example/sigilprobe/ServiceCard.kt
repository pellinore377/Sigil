package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*

@Composable
internal fun ServiceCard(value: ServiceContent) {
    var expanded by remember(value) { mutableStateOf(false) }
    var original by remember(value) { mutableStateOf(false) }
    var imperial by remember { mutableStateOf(false) }
    var selectedDay by remember(value) { mutableStateOf(value.today) }
    var selectedHour by remember(value, selectedDay) { mutableIntStateOf(0) }
    val unit = if (imperial) 1 else 0
    val clipboard = LocalClipboardManager.current
    val uri = LocalUriHandler.current
    val label = when(value.kind) { "translation"->"Translation"; "definition"->"Definition"; else->"Weather" }
    @Composable fun sense(s: DefinitionSense, full: Boolean) {
        RichMessageText(s.part, style = MaterialTheme.typography.labelMedium)
        RichMessageText(s.definition)
        if(full) {
            s.example?.let { Text("Example",style=MaterialTheme.typography.labelMedium); RichMessageText(it) }
            s.etymology?.let { Text("Origin",style=MaterialTheme.typography.labelMedium); RichMessageText(it) }
            if(s.synonyms.isNotEmpty()) { Text("Synonyms",style=MaterialTheme.typography.labelMedium); s.synonyms.forEach { RichMessageText(it) } }
            if(s.antonyms.isNotEmpty()) { Text("Antonyms",style=MaterialTheme.typography.labelMedium); s.antonyms.forEach { RichMessageText(it) } }
            SigilTextButton({ s.copy?.let { clipboard.setText(AnnotatedString(it)) } },enabled=s.copy!=null) { Text("Copy definition") }
        }
    }
    @Composable fun conditions(c: WeatherConditions, full: Boolean) {
        Text(c.date,style=MaterialTheme.typography.labelMedium)
        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
            Glyph(c.icon,36); Text(c.temperature[unit],style=MaterialTheme.typography.headlineMedium)
        }
        RichMessageText(c.description)
        c.feelsLike?.takeIf { it[unit]!=c.temperature[unit] }?.let { Text("Feels like ${it[unit]}",style=MaterialTheme.typography.bodyMedium) }
        if(c.rain!=null || c.chance!=null) Text(listOfNotNull(c.chance?.let { "Precipitation $it" },c.rain).joinToString(" · "),style=MaterialTheme.typography.bodyMedium)
        Text("Wind ${c.wind[unit]}",style=MaterialTheme.typography.bodyMedium)
        if(full) {
            c.humidity?.let { Text("Humidity $it",style=MaterialTheme.typography.bodyMedium) }
            c.uv?.let { Text("UV index $it",style=MaterialTheme.typography.bodyMedium) }
        }
    }
    @Composable fun heading(full: Boolean) {
        if(value.language.isNotEmpty()) Text(value.language,style=MaterialTheme.typography.labelMedium)
        RichMessageText(value.title,style=if(value.kind=="translation") MaterialTheme.typography.bodyLarge else MaterialTheme.typography.titleLarge)
        value.pronunciation?.let { RichMessageText(it,style=MaterialTheme.typography.bodyMedium) }
        if(value.kind=="translation") {
            SigilTextButton({ original=!original }) { Text(if(original)"Hide original" else "Show original") }
            Expandable(original) { value.original?.let { RichMessageText(it) } }
        }
        if(value.kind=="weather") {
            if(value.historical) Text("Historical weather snapshot",style=MaterialTheme.typography.labelMedium)
            value.current?.let { conditions(it,full) }
            value.days.firstOrNull { it.key==value.today }?.let { Text("High ${it.high[unit]} · Low ${it.low[unit]}",style=MaterialTheme.typography.bodyMedium) }
            SigilTextButton({ imperial=!imperial }) { Text(if(imperial)"Use °C and km/h" else "Use °F and mph") }
        }
    }
    @Composable fun attribution(full: Boolean) {
        RichMessageText(value.attribution,style=MaterialTheme.typography.labelSmall)
        Text("Snapshot ${value.stamp}",style=MaterialTheme.typography.labelSmall)
        if(full) value.source?.let { SigilTextButton({ uri.openUri(it) }) { Text("Open source") } }
    }
    Column(Modifier.widthIn(min=200.dp,max=280.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Text(label,style=MaterialTheme.typography.labelMedium)
        heading(false)
        value.senses.firstOrNull()?.let { sense(it,false) }
        attribution(false)
        SigilTextButton({ expanded=true }) { Glyph("open_in_full",18); Spacer(Modifier.width(8.dp)); Text(if(value.senses.size>1)"More definitions" else "Open ${label.lowercase()}") }
    }
    if(expanded) Dialog({ expanded=false },DialogProperties(usePlatformDefaultWidth=false)) {
        Surface(Modifier.fillMaxSize()) {
            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface) {
                Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    Row(verticalAlignment=Alignment.CenterVertically) {
                        SigilIconButton({ expanded=false }) { Glyph("close",24,"Close ${label.lowercase()}") }
                        Text(label,Modifier.weight(1f),style=MaterialTheme.typography.titleLarge)
                        if(value.kind=="translation") SigilIconButton({ value.copy?.let { clipboard.setText(AnnotatedString(it)) } },enabled=value.copy!=null) { Glyph("content_copy",24,"Copy translation") }
                    }
                    LazyColumn(Modifier.weight(1f).fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(16.dp)) {
                        item { Column(verticalArrangement=Arrangement.spacedBy(10.dp)) { heading(true) } }
                        items(value.senses) { Column(verticalArrangement=Arrangement.spacedBy(8.dp)) { sense(it,true) } }
                        if(value.audio!=null) item { SigilTextButton({ uri.openUri(value.audio) }) { Glyph("volume_up",20); Text("Open pronunciation audio") } }
                        if(value.days.isNotEmpty()) item {
                            Text("Forecast",style=MaterialTheme.typography.titleMedium)
                            LazyRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) { items(value.days) { day ->
                                Surface(onClick={selectedDay=day.key},shape=MaterialTheme.shapes.medium,color=if(selectedDay==day.key)MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surfaceVariant) {
                                    Column(Modifier.width(180.dp).padding(14.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                                        Text(day.date,style=MaterialTheme.typography.labelMedium); Glyph(day.icon,28)
                                        Text("${day.high[unit]} / ${day.low[unit]}"); RichMessageText(day.description,style=MaterialTheme.typography.bodyMedium); Text("Precipitation ${day.chance}",style=MaterialTheme.typography.labelMedium)
                                    }
                                }
                            } }
                        }
                        if(value.kind=="weather") {
                            val hours=value.hours.filter { it.key==selectedDay }
                            item { Text(if(hours.isEmpty())"No hourly forecast supplied for this day." else "Hourly forecast",style=MaterialTheme.typography.titleMedium) }
                            value.days.firstOrNull { it.key==selectedDay }?.charts?.getOrNull(unit)?.let { chart -> item {
                                ChartPlot(chart,emptySet(),selectedHour,{selectedHour=it},Modifier.fillMaxWidth().height(220.dp))
                            } }
                            hours.getOrNull(selectedHour)?.let { c -> item { Column(verticalArrangement=Arrangement.spacedBy(8.dp)) {
                                conditions(c,true)
                                Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                                    SigilTextButton({selectedHour--},enabled=selectedHour>0) { Text("Previous hour") }
                                    SigilTextButton({selectedHour++},enabled=selectedHour<hours.lastIndex) { Text("Next hour") }
                                }
                            } } }
                        }
                        item { Column(verticalArrangement=Arrangement.spacedBy(8.dp)) { attribution(true) } }
                    }
                }
            }
        }
    }
}
