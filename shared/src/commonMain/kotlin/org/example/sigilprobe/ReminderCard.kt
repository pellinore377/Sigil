package org.sigil

import androidx.compose.animation.*
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

// One line of identity, one line of time: the bell states the kind, so no type word is printed.
@Composable internal fun ReminderCard(part:MessagePart,analyze:(String)->String) {
    val motion=LocalMotion.current
    val now=temporalNow(part.at,3600L)
    val fired=part.at in 1..now
    CardColumn {
        if(part.at<=0) {Text("No reminder time set.",style=MaterialTheme.typography.bodyMedium);return@CardColumn}
        Row(horizontalArrangement=Arrangement.spacedBy(12.dp),verticalAlignment=Alignment.Top) {
            AnimatedContent(fired,transitionSpec={(fadeIn(motion.enter(MotionInline))+scaleIn(motion.enter(MotionInline),initialScale=.8f)) togetherWith fadeOut(motion.exit(MotionExit))},label="Reminder state") {done->
                Box(Modifier.size(38.dp).clip(CircleShape).background(LocalContentColor.current.copy(alpha=if(done).09f else .14f)),contentAlignment=Alignment.Center) {
                    Glyph(if(done)"check" else "notifications",20,"Reminder")
                }
            }
            Column(Modifier.weight(1f),verticalArrangement=Arrangement.spacedBy(3.dp)) {
                // An untitled reminder shows only its bell; the type word is never printed.
                if(part.rich!=null)RichMessageText(part.rich,style=MaterialTheme.typography.titleMedium)
                else if(part.text.isNotBlank())MessageText(part.text,analyze)
                Text(listOf(part.date,if(fired)"rang ${temporalSpan(now-part.at)} ago" else "in ${temporalSpan(maxOf(part.at-now,60L))}").filter {it.isNotEmpty()}.joinToString(" · "),
                    style=MaterialTheme.typography.labelMedium,color=LocalContentColor.current.copy(alpha=.68f),maxLines=2,overflow=TextOverflow.Ellipsis)
            }
        }
    }
}
