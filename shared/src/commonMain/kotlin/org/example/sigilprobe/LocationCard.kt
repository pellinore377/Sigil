package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

@Composable internal fun LocationCard(message:ChatMessage,part:MessagePart,analyze:(String)->String,command:Command?,bare:Boolean) {
    val caption=part.text.takeIf {it.isNotBlank() && it !in listOf("My location","Dropped pin")}
    CardColumn {
        LocalLocationContent.current(message,part,command)
        if(caption!=null) {
            val body:@Composable ()->Unit={if(part.rich!=null)RichMessageText(part.rich) else MessageText(caption,analyze)}
            if(bare)Surface(Modifier.padding(top=4.dp),shape=RoundedCornerShape(16.dp),color=if(message.mine)MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
                contentColor=if(message.mine)MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant) {
                Box(Modifier.padding(horizontal=14.dp,vertical=10.dp)) {body()}
            } else body()
        }
    }
}

// Stands in for the map on clients without a renderer: same surface, marker and countdown, no coordinates.
@Composable internal fun LocationSurface(message:ChatMessage,part:MessagePart,command:Command?) {
    val live=part.locationMode=="live"
    val now=temporalNow(if(part.stopped)0L else part.until ?: 0L,settles=true)
    val active=live && !part.stopped && part.until?.let {now<it}==true
    val title=when(part.locationMode) {"live"->if(active)"Live location" else "Location sharing ended";"once"->"Shared location";else->"Dropped pin"}
    Column(verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Box(Modifier.fillMaxWidth().heightIn(min=160.dp).clip(RoundedCornerShape(16.dp)).background(LocalContentColor.current.copy(alpha=.08f)).semantics {contentDescription=title},contentAlignment=Alignment.Center) {
            Column(Modifier.padding(horizontal=20.dp,vertical=16.dp),horizontalAlignment=Alignment.CenterHorizontally,verticalArrangement=Arrangement.spacedBy(8.dp)) {
                if(live || part.locationMode=="once")LocationAvatar(LocalMediaSender.current(message),message.author,active && now>=part.sampledAt && now-part.sampledAt<=60)
                else Glyph("place",36)
                Text("Map unavailable on this device.",style=MaterialTheme.typography.bodyMedium,textAlign=TextAlign.Center,maxLines=2,overflow=TextOverflow.Ellipsis)
            }
            if(live)LocationMapChip(locationRemaining(part.until,now,part.stopped),Modifier.align(Alignment.TopStart).padding(10.dp))
        }
        if(active && part.canStop && command!=null)SigilTextButton({command("location_stop",mapOf("peer" to message.peer,"author" to message.author,"message" to message.id,"card" to part.id))}) {
            Glyph("location_off",18);Spacer(Modifier.width(8.dp));Text("Stop sharing")
        }
    }
}
