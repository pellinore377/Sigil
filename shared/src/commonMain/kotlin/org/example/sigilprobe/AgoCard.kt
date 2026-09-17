package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable

@Composable internal fun AgoCard(part:MessagePart,analyze:(String)->String) {
    val now=temporalNow(part.at)
    TemporalFrame("history","Elapsed time",part,analyze) {
        if(part.at>0) {
            temporalScale(now-part.at).let {(count,unit)->TemporalFigure("$count","$unit ago")}
            TemporalCaption(temporalBreakdown(part.at,now))
            TemporalCaption(if(part.date.isEmpty())"" else "Since ${part.date}")
        }
        else Text("No time set.",style=MaterialTheme.typography.bodyMedium)
    }
}
