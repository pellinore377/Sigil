package org.sigil

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun ServiceCard(value: ServiceContent) {
    var imperial by remember { mutableStateOf(false) }
    val unit = if (imperial) 1 else 0
    val uri = LocalUriHandler.current
    val quiet = LocalContentColor.current.copy(alpha = .70f)
    val label = when(value.kind) { "translation"->"Translation"; "definition"->"Definition"; else->"Weather" }
    val icon = when(value.kind) { "translation"->"translate"; "definition"->"dictionary"; else->value.current?.icon ?: "partly_cloudy_day" }
    @Composable fun quietly(content: @Composable ()->Unit) = CompositionLocalProvider(LocalContentColor provides quiet,content=content)
    @Composable fun metric(name: String, reading: String) {
        Column(Modifier.widthIn(min=72.dp),verticalArrangement=Arrangement.spacedBy(2.dp)) {
            Text(name,style=MaterialTheme.typography.labelSmall,color=quiet,maxLines=1,overflow=TextOverflow.Ellipsis)
            Text(reading,style=MaterialTheme.typography.bodyMedium,maxLines=1,overflow=TextOverflow.Ellipsis)
        }
    }
    @Composable fun sense(index: Int, entry: DefinitionSense, part: Boolean) {
        if(part) quietly { RichMessageText(entry.part,style=MaterialTheme.typography.labelMedium) }
        Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            if(value.senses.size>1) Text("${index+1}.",Modifier.widthIn(min=16.dp),style=MaterialTheme.typography.bodyLarge,color=quiet)
            Column(verticalArrangement=Arrangement.spacedBy(4.dp)) {
                RichMessageText(entry.definition)
                entry.example?.let { quietly { RichMessageText(it,style=MaterialTheme.typography.bodyMedium) } }
            }
        }
    }
    @Composable fun conditions(c: WeatherConditions) {
        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
            Text(c.temperature[unit],style=MaterialTheme.typography.headlineMedium,maxLines=1,overflow=TextOverflow.Ellipsis)
            Column(Modifier.weight(1f),verticalArrangement=Arrangement.spacedBy(2.dp)) {
                RichMessageText(c.description,style=MaterialTheme.typography.bodyMedium)
                value.days.firstOrNull { it.key==value.today }?.let { Text("High ${it.high[unit]} · Low ${it.low[unit]}",style=MaterialTheme.typography.labelMedium,color=quiet) }
            }
        }
        FlowRow(horizontalArrangement=Arrangement.spacedBy(12.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            c.chance?.let { metric("Rain chance",it) }
            metric("Wind",c.wind[unit])
            c.feelsLike?.let { metric("Feels like",it[unit]) }
            c.rain?.let { metric("Rainfall",it) }
            c.humidity?.let { metric("Humidity",it) }
            c.uv?.let { metric("UV index",it) }
        }
    }
    @Composable fun forecast() {
        Row(Modifier.horizontalScroll(rememberScrollState()),horizontalArrangement=Arrangement.spacedBy(12.dp)) {
            value.days.forEach { day ->
                Column(Modifier.widthIn(min=56.dp),horizontalAlignment=Alignment.CenterHorizontally,verticalArrangement=Arrangement.spacedBy(4.dp)) {
                    Text(day.date.substringBefore(','),style=MaterialTheme.typography.labelSmall,color=quiet,maxLines=1,overflow=TextOverflow.Ellipsis)
                    Glyph(day.icon,20)
                    Text(day.high[unit],style=MaterialTheme.typography.bodyMedium,maxLines=1,overflow=TextOverflow.Ellipsis)
                    Text(day.low[unit],style=MaterialTheme.typography.bodyMedium,color=quiet,maxLines=1,overflow=TextOverflow.Ellipsis)
                    Text(day.chance,style=MaterialTheme.typography.labelSmall,color=quiet,maxLines=1,overflow=TextOverflow.Ellipsis)
                }
            }
        }
    }
    Column(Modifier.widthIn(min=200.dp,max=280.dp).animateContentSize(LocalMotion.current.tween(MotionMillis)),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) { Glyph(icon,20);Text(label,style=MaterialTheme.typography.labelMedium) }
        RichMessageText(value.title,style=if(value.kind=="definition") MaterialTheme.typography.titleLarge else MaterialTheme.typography.titleMedium)
        if(value.kind=="translation") value.original?.let { quietly { RichMessageText(it,style=MaterialTheme.typography.bodyMedium) } }
        if(value.kind=="definition") FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
            value.pronunciation?.let { quietly { RichMessageText(it,style=MaterialTheme.typography.bodyMedium) } }
            value.senses.firstOrNull()?.let { RichMessageText(it.part,style=MaterialTheme.typography.bodyMedium) }
            value.audio?.let { audio->Symbol("volume_up","Play pronunciation") { uri.openUri(audio) } }
        }
        if(value.language.isNotEmpty()) Text(value.language,style=MaterialTheme.typography.labelMedium,color=quiet,maxLines=1,overflow=TextOverflow.Ellipsis)
        if(value.kind=="weather") {
            if(value.historical) Text("Historical weather snapshot",style=MaterialTheme.typography.labelMedium,color=quiet)
            value.current?.let { conditions(it) }
            if(value.days.isNotEmpty()) forecast()
            if(value.current!=null || value.days.isNotEmpty()) SigilTextButton({ imperial=!imperial }) { Glyph("swap_horiz",18);Spacer(Modifier.width(8.dp));Text(if(imperial)"Use °C and km/h" else "Use °F and mph") }
        }
        value.senses.forEachIndexed { index,entry-> sense(index,entry,index>0 && entry.part.text!=value.senses[index-1].part.text) }
        quietly { RichMessageText(value.attribution,style=MaterialTheme.typography.labelSmall) }
        if(value.kind!="weather") value.source?.let { SigilTextButton({ uri.openUri(it) }) { Glyph("open_in_new",18);Spacer(Modifier.width(8.dp));Text("Open source") } }
    }
}
