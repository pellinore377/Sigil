package org.sigil

import androidx.compose.animation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.FirstBaseline
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlin.math.abs
import kotlin.math.roundToInt

private fun reminderUnit(count:Long,name:String)=if(count==1L)"1 $name" else "$count ${name}s"
// Minutes within the hour, hours and minutes within the day, then one whole unit.
internal fun reminderSpan(at:Long,now:Long):String {
    val distance=abs(at-now)
    val span=when {
        distance<60L->return if(at<=now)"Rang just now" else "In under a minute"
        distance<3600L->reminderUnit(distance/60L,"minute")
        distance<86400L->temporalSpan(distance-distance%60L)
        else->temporalSpan(distance).split(' ').take(2).joinToString(" ")
    }
    return if(at<=now)"Rang $span ago" else "In $span"
}

// The bell carries the state (filled while upcoming); the time reads in the viewer's own calendar and clock, the span under it.
@Composable internal fun ReminderCard(part:MessagePart,analyze:(String)->String) {
    val now=temporalNow(part.at,60L)
    val motion=LocalMotion.current
    val density=LocalDensity.current
    val title=MaterialTheme.typography.titleMedium
    val quiet=LocalContentColor.current.copy(alpha=.68f)
    Column(Modifier.widthIn(min=MessageCardMinWidth,max=MessageCardMaxWidth).padding(vertical=4.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
        if(part.at<=0) {Text("No reminder time set.",style=MaterialTheme.typography.bodyMedium);return@Column}
        val fired=part.at<=now
        Row(horizontalArrangement=Arrangement.spacedBy(12.dp)) {
            Box(Modifier.size(24.dp).alignBy {it.measuredHeight/2+with(density){(title.fontSize*.3f).toPx().roundToInt()}},contentAlignment=Alignment.Center) {
                AnimatedContent(fired,transitionSpec={(fadeIn(motion.enter(MotionInline))+scaleIn(motion.enter(MotionInline),initialScale=.8f)) togetherWith fadeOut(motion.exit(MotionExit))},label="Reminder state") {
                    CompositionLocalProvider(LocalContentColor provides if(it)quiet else LocalContentColor.current) {Glyph("notifications",20,filled=!it)}
                }
            }
            // An untitled reminder shows only its bell; the type word is never printed.
            CardTitle(part,listOf("Reminder",part.text).filter {it.isNotBlank()}.joinToString(". "),Modifier.weight(1f).alignBy(FirstBaseline),if(fired)"Rang" else "Upcoming")
        }
        if(part.date.isNotBlank())Text(part.date,Modifier.padding(top=8.dp),style=MaterialTheme.typography.bodyMedium.copy(fontFeatureSettings="tnum, lnum"),maxLines=2,overflow=TextOverflow.Ellipsis)
        Text(reminderSpan(part.at,now),style=MaterialTheme.typography.labelMedium.copy(fontFeatureSettings="tnum, lnum"),color=quiet,maxLines=1)
    }
}
