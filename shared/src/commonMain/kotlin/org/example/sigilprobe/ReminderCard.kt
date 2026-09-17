package org.sigil

import androidx.compose.animation.*
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

@Composable internal fun ReminderCard(part:MessagePart,analyze:(String)->String) {
    val motion=LocalMotion.current
    val now=temporalNow(part.at,120L)
    val fired=part.at in 1..now
    val settled by animateFloatAsState(if(fired).75f else 1f,motion.tween(MotionMillis),label="Reminder completion")
    CardFrame("notifications_active","Reminder") {
        if(part.rich!=null)RichMessageText(part.rich,style=MaterialTheme.typography.titleMedium)else MessageText(part.text,analyze)
        if(part.at>0)AnimatedContent(fired,transitionSpec={(fadeIn(motion.enter(MotionMillis))+scaleIn(motion.enter(MotionMillis),initialScale=.96f)) togetherWith fadeOut(motion.exit(MotionExit)) using SizeTransform(false) {_,_->motion.tween(MotionMillis)}},label="Reminder state") {done->
            Column(verticalArrangement=Arrangement.spacedBy(4.dp)) {
                if(done)Row(Modifier.alpha(settled),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    Glyph("check_circle",20)
                    Text("Reminded ${temporalSpan(now-part.at)} ago",style=MaterialTheme.typography.bodyMedium,maxLines=2,overflow=TextOverflow.Ellipsis)
                }
                else temporalScale(maxOf(part.at-now,60L)).let {(count,unit)->TemporalFigure("$count","$unit from now")}
                TemporalCaption(if(part.date.isEmpty())"" else if(done)part.date else "Due ${part.date}")
            }
        }
        else Text("No reminder time set.",style=MaterialTheme.typography.bodyMedium)
    }
}
