package org.sigil

import androidx.compose.animation.*
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

@Composable internal fun ReminderCard(part:MessagePart,analyze:(String)->String) {
    val motion=LocalMotion.current
    val now=temporalNow(part.at,120L)
    val fired=part.at in 1..now
    val settled by animateFloatAsState(if(fired).75f else 1f,motion.tween(MotionMillis),label="Reminder completion")
    TemporalFrame(if(fired)"check_circle" else "notifications_active","Reminder",part,analyze) {
        if(part.at>0)AnimatedContent(fired,transitionSpec={(fadeIn(motion.enter(MotionMillis))+scaleIn(motion.enter(MotionMillis),initialScale=.96f)) togetherWith fadeOut(motion.exit(MotionExit)) using SizeTransform(false) {_,_->motion.tween(MotionMillis)}},label="Reminder state") {done->
            Column(Modifier.alpha(if(done)settled else 1f),verticalArrangement=Arrangement.spacedBy(2.dp)) {
                if(part.date.isNotEmpty())Text(part.date,style=MaterialTheme.typography.bodyMedium,maxLines=2,overflow=TextOverflow.Ellipsis)
                Text(if(done)"Reminded ${temporalSpan(now-part.at)} ago" else "in ${temporalSpan(maxOf(part.at-now,60L))}",style=MaterialTheme.typography.labelSmall,maxLines=2,overflow=TextOverflow.Ellipsis)
            }
        }
        else Text("No reminder time set.",style=MaterialTheme.typography.bodyMedium)
    }
}
