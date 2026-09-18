package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

// A note sits in the bubble it arrived in; the glyph is its only mark, centred on the text.
@Composable internal fun NoteCard(part:MessagePart,analyze:(String)->String) {
    Row(horizontalArrangement=Arrangement.spacedBy(10.dp),verticalAlignment=Alignment.CenterVertically) {
        Glyph("sticky_note_2",18,"Note")
        Box(Modifier.weight(1f,fill=false)) {if(part.rich!=null)RichMessageText(part.rich) else MessageText(part.text,analyze)}
    }
}
