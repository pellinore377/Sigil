package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

@Composable
internal fun ContactCard(contact: ContactContent,open: (() -> Unit)?) {
    val ink=LocalContentColor.current
    val type=MaterialTheme.typography
    val hidden=contact.name.spans.any { it.reveal.isNotEmpty() }
    val who=if(hidden) "hidden name" else contact.name.text.trim().ifEmpty { contact.address }
    Column(Modifier.widthIn(min=MessageCardMinWidth,max=MessageCardMaxWidth).width(IntrinsicSize.Max).padding(vertical=4.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        Row(Modifier.fillMaxWidth().clearAndSetSemantics { contentDescription="Contact, $who, ${contact.address}" },verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
            ContactMonogram(contact.name)
            Column(Modifier.weight(1f),verticalArrangement=Arrangement.spacedBy(4.dp)) {
                RichMessageText(contact.name,style=type.titleMedium,maxLines=3)
                Text(contact.address,style=type.labelMedium,color=ink.copy(alpha=.68f),maxLines=2,overflow=TextOverflow.Ellipsis)
            }
        }
        if(open!=null) CardInkAction("Message",open,Modifier.fillMaxWidth().semantics { contentDescription=if(hidden) "Message this contact" else "Message $who" })
    }
}

// Letters only: an emoji or symbol would split a surrogate pair or read as noise, so those words are skipped.
internal fun contactInitials(name: String)=name.split(Regex("\\s+")).mapNotNull { it.firstOrNull()?.takeIf(Char::isLetter) }.take(2).joinToString("") { it.uppercase() }

// Initials would leak a concealed name, so a hidden name gets the person glyph.
@Composable private fun ContactMonogram(name: RichText) {
    val ink=LocalContentColor.current
    val initials=contactInitials(name.text)
    Box(Modifier.size(48.dp).clip(CircleShape).background(ink.copy(alpha=.12f)).clearAndSetSemantics {},contentAlignment=Alignment.Center) {
        if(name.spans.any {it.reveal.isNotEmpty()} || initials.isEmpty()) Glyph("person",22)
        else Text(initials,style=MaterialTheme.typography.titleMedium,maxLines=1)
    }
}
