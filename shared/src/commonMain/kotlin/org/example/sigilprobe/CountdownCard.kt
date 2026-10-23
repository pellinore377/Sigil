package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlin.math.abs

// Days between the sender's target day and now; positive is past. Without a day start, whole days from the instant stand in.
internal fun distanceDays(at:Long,dayStart:Long,now:Long)=if(dayStart>0)(now-dayStart).floorDiv(86400L) else (now-at)/86400L

// The sender's civil day; without the core's ISO date, the day start's UTC noon is close enough.
internal fun senderDay(iso:String,dayStart:Long,at:Long)=isoDay(iso) ?: (if(dayStart>0)dayStart+43200L else at).floorDiv(86400L)

// The figure and its unit: Today, whole days, then years once a year is worth naming. Never a negative count.
internal fun distanceFigure(days:Long):Pair<String,String> {
    if(days==0L)return "Today" to "the date is here"
    val (count,unit)=temporalCalendarScale(abs(days)*86400L)
    return "$count" to "$unit ${if(days>0)"ago" else "to go"}"
}

// Within a day of a timed moment: nearest hour, then minutes under one (ahead rounds up, past down).
internal fun nearFigure(at:Long,now:Long):Pair<String,String>? {
    val delta=at-now
    val span=abs(delta)
    if(span>=86400L)return null
    val (size,name)=if(span<3600L)60L to "minute" else 3600L to "hour"
    val count=maxOf(1L,if(size==3600L)(span+1800L)/3600L else if(delta>0)(span+59L)/60L else span/60L)
    return "$count" to "$name${if(count==1L)"" else "s"} ${if(delta>0)"to go" else "ago"}"
}

// Elapsed time reads whole calendar days in the sender's calendar, and at most two units after the lead.
internal fun elapsedFigure(fromDay:Long,toDay:Long):Pair<String,String>? {
    val lead=calendarSteps(fromDay,toDay).firstOrNull() ?: return null
    return "${lead.first}" to "${lead.second}${if(lead.first==1L)"" else "s"} ago"
}
internal fun elapsedBreakdown(fromDay:Long,toDay:Long)=temporalPhrase(calendarSteps(fromDay,toDay).drop(1).take(2))

@Composable internal fun CountdownCard(part:MessagePart,analyze:(String)->String)=DistanceCard(part,analyze,elapsed=false)
@Composable internal fun AgoCard(part:MessagePart,analyze:(String)->String)=DistanceCard(part,analyze,elapsed=true)

// A countdown becomes Today on the sender's date, then elapsed time; an elapsed card dated ahead counts down instead.
@Composable private fun DistanceCard(part:MessagePart,analyze:(String)->String,elapsed:Boolean) {
    val now=temporalNow(part.at,0L)
    val days=distanceDays(part.at,part.dayStart,now)
    Column(Modifier.widthIn(min=MessageCardMinWidth,max=MessageCardMaxWidth).padding(vertical=4.dp)) {
        if(part.at<=0) {Text(if(elapsed)"No origin date set." else "No target date set.",style=MaterialTheme.typography.bodyMedium);return@Column}
        val label=if(elapsed)"Elapsed time" else "Countdown"
        val ahead=if(part.timed)part.at>now else days<0
        TemporalHeader(if(elapsed)"history" else if(ahead)"calendar_month" else "event_available",label,part,analyze,fill=false)
        val near=if(part.timed)nearFigure(part.at,now) else null
        val from=senderDay(part.dateIso,part.dayStart,part.at)
        val past=if(near==null && elapsed && days>0)elapsedFigure(from,from+days) else null
        val (figure,unit)=near ?: past ?: distanceFigure(days)
        TemporalFigure(figure,unit,Modifier.padding(top=12.dp))
        val breakdown=if(past!=null)elapsedBreakdown(from,from+days) else ""
        val since=elapsed && !ahead && days>0 && part.date.isNotEmpty()
        val meta=listOf(breakdown,if(since)"${if(breakdown.isEmpty())"Since" else "since"} ${part.date}" else part.date).filter {it.isNotEmpty()}.joinToString(" · ")
        if(meta.isNotEmpty())Text(meta,Modifier.padding(top=8.dp),style=MaterialTheme.typography.labelMedium.copy(fontFeatureSettings="tnum, lnum"),
            color=LocalContentColor.current.copy(alpha=.68f),maxLines=2,overflow=TextOverflow.Ellipsis)
    }
}
