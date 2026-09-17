@file:OptIn(kotlin.time.ExperimentalTime::class)
package org.sigil

import androidx.compose.animation.animateContentSize
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
            val step=(if(distance<fine)1L else if(distance<86400L)60L else 3600L)*1000L
            delay(step-instant.toEpochMilliseconds().mod(step))
        }
    }
    return now
}

@Composable internal fun TemporalCard(icon:String,label:String,content:@Composable ColumnScope.()->Unit) {
    Column(Modifier.widthIn(min=200.dp,max=280.dp).animateContentSize(LocalMotion.current.tween(MotionMillis)),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {Glyph(icon,20);Text(label,style=MaterialTheme.typography.labelMedium)}
        content()
    }
}
@Composable internal fun TemporalFigure(value:String,unit:String,modifier:Modifier=Modifier) {
    Row(modifier.clearAndSetSemantics {contentDescription=if(unit.isEmpty())value else "$value $unit"},horizontalArrangement=Arrangement.spacedBy(8.dp)) {
        Text(value,Modifier.alignByBaseline(),style=MaterialTheme.typography.headlineMedium,fontFamily=LocalCodeFont.current,maxLines=1)
        if(unit.isNotEmpty())Text(unit,Modifier.alignByBaseline().weight(1f,false),style=MaterialTheme.typography.labelMedium,maxLines=2,overflow=TextOverflow.Ellipsis)
    }
}
@Composable internal fun TemporalCaption(text:String) {
    if(text.isNotBlank())Text(text,style=MaterialTheme.typography.labelSmall,maxLines=3,overflow=TextOverflow.Ellipsis)
}
