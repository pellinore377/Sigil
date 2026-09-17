package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable

@Composable internal fun AgoCard(part:MessagePart,analyze:(String)->String) {
    val now=temporalNow(part.at)
    TemporalCard("history","Elapsed time") {
        if(part.rich!=null)RichMessageText(part.rich,style=MaterialTheme.typography.titleMedium)else MessageText(part.text,analyze)
        if(part.at>0) {
            temporalScale(now-part.at).let {(count,unit)->TemporalFigure("$count","$unit ago")}
            TemporalCaption(if(part.date.isEmpty())"" else "Since ${part.date}")
        }
    }
}
