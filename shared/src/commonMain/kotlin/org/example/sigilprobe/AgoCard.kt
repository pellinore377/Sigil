package org.sigil

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable

@Composable internal fun AgoCard(part:MessagePart,analyze:(String)->String) {
    val now=temporalNow(part.at)
    CardColumn {
        if(part.at<=0) {Text("No origin date set.",style=MaterialTheme.typography.bodyMedium);return@CardColumn}
        TemporalHeader("history","Elapsed time",part,analyze)
        // The leading unit carries the figure; the rest of the calendar breakdown follows it.
        val steps=temporalComponents(part.at,now)
        val lead=steps.firstOrNull()
        if(lead==null)TemporalFigure("Today","the date is here")
        else {
            TemporalFigure("${lead.first}","${lead.second}${if(lead.first==1L)"" else "s"} ago")
            TemporalCaption(temporalPhrase(steps.drop(1)))
        }
    }
}
