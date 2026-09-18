package org.sigil

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable

@Composable internal fun CountdownCard(part:MessagePart,analyze:(String)->String) {
    // Within a day the target is close enough to read out live, so the tick refines to the second.
    val now=temporalNow(part.at,86400L)
    val left=part.at-now
    CardColumn {
        if(part.at<=0) {Text("No target date set.",style=MaterialTheme.typography.bodyMedium);return@CardColumn}
        TemporalHeader(if(left>0)"calendar_month" else "event_available","Countdown",part,analyze)
        if(left>=86400L)temporalCalendarScale(left).let {(count,unit)->TemporalFigure("$count","$unit to go")}
        else if(left>0L)TemporalFigure(temporalDigits(left),"to go")
        else if(-left<86400L)TemporalFigure("Today","the date is here")
        else temporalCalendarScale(-left).let {(count,unit)->TemporalFigure("$count","$unit ago")}
    }
}
