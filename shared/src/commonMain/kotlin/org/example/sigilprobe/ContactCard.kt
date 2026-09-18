package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

@Composable
internal fun ContactCard(contact: ContactContent,open: (() -> Unit)?) {
    val nameCap=with(LocalDensity.current) { MaterialTheme.typography.titleMedium.lineHeight.toDp()*2 }
    Column(Modifier.widthIn(min=MessageCardMinWidth,max=MessageCardMaxWidth),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) { Glyph("person",20);Text("Contact",style=MaterialTheme.typography.labelMedium) }
        Row(Modifier.fillMaxWidth(),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
            ContactAvatar(contact.name,48)
            Column(Modifier.weight(1f),verticalArrangement=Arrangement.spacedBy(2.dp)) {
                RichMessageText(contact.name,Modifier.heightIn(max=nameCap).clipToBounds(),MaterialTheme.typography.titleMedium)
                Text(contact.address,style=MaterialTheme.typography.bodySmall,color=LocalContentColor.current.copy(alpha=.7f),maxLines=1,overflow=TextOverflow.Ellipsis)
            }
            if(open!=null) SigilTextButton(open,modifier=Modifier.semantics { contentDescription="Message shared contact" }) { Glyph("chat_bubble",18);Spacer(Modifier.width(8.dp));Text("Message") }
        }
    }
}

// Initials would leak a concealed name into the avatar and its description.
@Composable private fun ContactAvatar(name: RichText,size: Int) {
    if(name.spans.any {it.reveal.isNotEmpty()}) Box(Modifier.size(size.dp).clip(CircleShape).background(LocalContentColor.current.copy(alpha=.12f)),contentAlignment=Alignment.Center) { Glyph("person",(size*.45f).toInt(),"Hidden contact") }
    else Avatar(name.text,size)
}
