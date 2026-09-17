package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

internal fun noticeSummary(message:String):String = when {
    "(sqlite-5)" in message -> "Sync paused: device storage is busy."
    "authentication failed" in message.lowercase() || "crypto-authentication" in message -> "A message couldn’t be authenticated."
    message.startsWith("Sync:") -> message.substringBefore(" — ").substringBefore(" -- ").substringBefore(" - ").replaceFirst("Sync:","Couldn’t finish")
    else -> message.substringBefore(" Your stored keys").substringBefore(" Stored keys")
}

@Composable internal fun SyncNotice(message:String,dismiss:()->Unit) {
    var details by remember(message) {mutableStateOf(false)}
    Surface(Modifier.fillMaxWidth().testTag("sync-notice"),shape=RoundedCornerShape(20.dp),
        color=MaterialTheme.colorScheme.surfaceContainerHigh,shadowElevation=3.dp) {
        Column(Modifier.padding(start=16.dp,end=4.dp,top=4.dp,bottom=4.dp)) {
            Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                Glyph("error",20)
                Text(noticeSummary(message),Modifier.weight(1f).semantics {liveRegion=LiveRegionMode.Polite},
                    style=MaterialTheme.typography.bodyMedium,maxLines=2,overflow=TextOverflow.Ellipsis)
                Symbol(if(details)"expand_less" else "expand_more",if(details)"Hide error details" else "Show error details") {details=!details}
                Symbol("close","Dismiss notice",dismiss)
            }
            Expandable(details) {
                Text(message,Modifier.fillMaxWidth().heightIn(max=160.dp).verticalScroll(rememberScrollState()).padding(top=4.dp,end=12.dp,bottom=12.dp),
                    style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}
