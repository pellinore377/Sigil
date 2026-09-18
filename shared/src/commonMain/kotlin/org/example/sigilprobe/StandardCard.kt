package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

// Fallback for a card kind this build does not render; every known kind has its own card.
@Composable internal fun StandardCard(part:MessagePart,analyze:(String)->String) {
    CardFrame("data_object","Card") {
        if(part.rich!=null)RichMessageText(part.rich,style=MaterialTheme.typography.titleMedium)else MessageText(part.text,analyze)
        Column(verticalArrangement=Arrangement.spacedBy(4.dp)) {
            part.items.forEach {item->
                Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    Glyph(if(item.checked)"check_box" else "check_box_outline_blank",18)
                    if(item.rich!=null)RichMessageText(item.rich,Modifier.weight(1f),MaterialTheme.typography.bodyMedium)
                    else Text(item.text,Modifier.weight(1f),style=MaterialTheme.typography.bodyMedium,maxLines=3,overflow=TextOverflow.Ellipsis)
                }
            }
        }
    }
}
