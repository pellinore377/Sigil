@file:OptIn(kotlin.time.ExperimentalTime::class)
package org.sigil

import androidx.compose.animation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlin.math.abs

private val temporalLadder=listOf(31557600L to "year",2629800L to "month",86400L to "day",3600L to "hour",60L to "minute",1L to "second")
private fun temporalUnit(count:Long,name:String)=if(count==1L)"$count $name" else "$count ${name}s"

internal fun temporalScale(seconds:Long):Pair<Long,String> {
    val span=seconds.coerceAtLeast(0)
    for((size,name) in temporalLadder) {val count=span/size;if(count>0L)return count to if(count==1L)name else "${name}s"}
    return 0L to "seconds"
}
internal fun temporalSpan(seconds:Long):String {
    val span=seconds.coerceAtLeast(0)
    val index=temporalLadder.indexOfFirst {span/it.first>0L}
    if(index<0)return "0 seconds"
    val count=span/temporalLadder[index].first
    val rest=temporalLadder.getOrNull(index+1)?.let {(size,name)->((span-count*temporalLadder[index].first)/size).takeIf {it>0L}?.let {temporalUnit(it,name)}}
    return listOfNotNull(temporalUnit(count,temporalLadder[index].second),rest).joinToString(" ")
}
internal fun temporalClock(seconds:Long):Pair<String,String> {
    val span=seconds.coerceAtLeast(0)
    return if(span>=3600L)"${span/3600}:${((span%3600)/60).toString().padStart(2,'0')}" to "hours remaining"
    else "${span/60}:${(span%60).toString().padStart(2,'0')}" to "minutes remaining"
}
// Fixed cell count within a regime so the split-flap never rebuilds mid-second.
internal fun temporalDigits(seconds:Long):String {
    val span=seconds.coerceAtLeast(0)
    val body="${(span/60)%60}".let {if(span>=3600L)it.padStart(2,'0') else it}+":"+"${span%60}".padStart(2,'0')
    return if(span>=3600L)"${span/3600}:$body" else body
}
// Gregorian civil date (days since 1970-01-01); commonMain carries no date library.
private fun civilFromDays(days:Long):Triple<Long,Int,Int> {
    val shifted=days+719468
    val era=(if(shifted>=0)shifted else shifted-146096)/146097
    val dayOfEra=shifted-era*146097
    val yearOfEra=(dayOfEra-dayOfEra/1460+dayOfEra/36524-dayOfEra/146096)/365
    val dayOfYear=dayOfEra-(365*yearOfEra+yearOfEra/4-yearOfEra/100)
    val shiftedMonth=(5*dayOfYear+2)/153
    val month=(if(shiftedMonth<10)shiftedMonth+3 else shiftedMonth-9).toInt()
    return Triple(yearOfEra+era*400+if(month<=2)1L else 0L,month,(dayOfYear-(153*shiftedMonth+2)/5+1).toInt())
}
private fun monthLength(year:Long,month:Int)=when(month) {2->if(year%4==0L && (year%100!=0L || year%400==0L))29 else 28;4,6,9,11->30;else->31}
private fun daysFromCivil(year:Long,month:Int,day:Int):Long {
    val shifted=if(month<=2)year-1 else year
    val era=(if(shifted>=0)shifted else shifted-399)/400
    val yearOfEra=shifted-era*400
    val shiftedMonth=if(month>2)month-3 else month+9
    val dayOfYear=(153L*shiftedMonth+2)/5+day-1
    return era*146097+yearOfEra*365+yearOfEra/4-yearOfEra/100+dayOfYear-719468
}
// Adding a month clamps to the shorter month, so Jan 31 plus one month is the end of February.
private fun addMonths(year:Long,month:Int,day:Int,count:Long):Triple<Long,Int,Int> {
    val total=year*12+(month-1)+count
    val shiftedYear=total.floorDiv(12L)
    val shiftedMonth=total.mod(12L).toInt()+1
    return Triple(shiftedYear,shiftedMonth,minOf(day,monthLength(shiftedYear,shiftedMonth)))
}
// Calendar subtraction, so a years/months/days readout agrees with a calendar. Boundaries are UTC.
internal fun temporalBreakdown(from:Long,to:Long):String {
    if(to<=from)return ""
    var seconds=to.mod(86400L)-from.mod(86400L)
    var end=to.floorDiv(86400L)
    if(seconds<0L) {seconds+=86400L;end--}
    val (fromYear,fromMonth,fromDay)=civilFromDays(from.floorDiv(86400L))
    val (endYear,endMonth,_)=civilFromDays(end)
    var months=(endYear*12+endMonth)-(fromYear*12+fromMonth)
    while(months>0L && addMonths(fromYear,fromMonth,fromDay,months).let {daysFromCivil(it.first,it.second,it.third)}>end)months--
    val days=end-addMonths(fromYear,fromMonth,fromDay,months).let {daysFromCivil(it.first,it.second,it.third)}
    val steps=listOf(months/12 to "year",months%12 to "month",days to "day",seconds/3600 to "hour",(seconds%3600)/60 to "minute").filter {it.first>0L}
    return if(steps.isEmpty())"Less than a minute" else steps.take(4).joinToString(" · ") {temporalUnit(it.first,it.second)}
}

// Ticks on the wall-clock boundary, coarsening with distance; settles from absolute time when brought back into view.
@Composable internal fun temporalNow(target:Long,fine:Long=3600L,settles:Boolean=false):Long {
    val visible=LocalMotionVisible.current
    var now by remember(target) {mutableLongStateOf(kotlin.time.Clock.System.now().epochSeconds)}
    LaunchedEffect(target,fine,settles,visible) {
        while(visible) {
            val instant=kotlin.time.Clock.System.now()
            now=instant.epochSeconds
            if(settles && now>=target)break
            val distance=abs(target-now)
            val step=(if(distance<fine)1L else 60L)*1000L
            delay(step-instant.toEpochMilliseconds().mod(step))
        }
    }
    return now
}

@Composable internal fun CardColumn(content:@Composable ColumnScope.()->Unit) {
    Column(Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).animateContentSize(LocalMotion.current.tween(MotionMillis)),verticalArrangement=Arrangement.spacedBy(8.dp),content=content)
}
@Composable internal fun CardFrame(icon:String,label:String,content:@Composable ColumnScope.()->Unit) {
    CardColumn {
        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {Glyph(icon,20);Text(label,style=MaterialTheme.typography.labelMedium)}
        content()
    }
}
// Owner-ordered for the temporal family: the title takes line 1 beside the glyph, not the type word.
@Composable internal fun TemporalFrame(icon:String,label:String,part:MessagePart,analyze:(String)->String,content:@Composable ColumnScope.()->Unit) {
    val motion=LocalMotion.current
    CardColumn {
        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            AnimatedContent(icon,transitionSpec={(fadeIn(motion.enter(MotionInline))+scaleIn(motion.enter(MotionInline),initialScale=.8f)) togetherWith fadeOut(motion.exit(MotionExit))},label="$label state") {Glyph(it,20,label)}
            Box(Modifier.weight(1f)) {
                if(part.rich!=null)RichMessageText(part.rich,style=MaterialTheme.typography.titleMedium)
                else if(part.text.isNotBlank())MessageText(part.text,analyze)
                else Text(label,style=MaterialTheme.typography.titleMedium)
            }
        }
        content()
    }
}
@Composable internal fun TemporalFigure(value:String,unit:String,modifier:Modifier=Modifier) {
    Row(modifier.clearAndSetSemantics {contentDescription=if(unit.isEmpty())value else "$value $unit"},horizontalArrangement=Arrangement.spacedBy(8.dp)) {
        Text(value,Modifier.alignByBaseline(),style=MaterialTheme.typography.titleLarge,fontFamily=LocalCodeFont.current,maxLines=1)
        if(unit.isNotEmpty())Text(unit,Modifier.alignByBaseline().weight(1f,false),style=MaterialTheme.typography.labelMedium,maxLines=1,overflow=TextOverflow.Ellipsis)
    }
}
@Composable internal fun TemporalCaption(text:String) {
    if(text.isNotBlank())Text(text,style=MaterialTheme.typography.labelSmall,maxLines=3,overflow=TextOverflow.Ellipsis)
}
