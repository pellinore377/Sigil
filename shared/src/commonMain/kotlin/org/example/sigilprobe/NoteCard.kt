package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

// The notes-grid tile itself: same shape and surface role as Inbox's note tile, plus its leading glyph.
@Composable internal fun NoteCard(part:MessagePart,analyze:(String)->String) {
    CardColumn {
        Surface(Modifier.fillMaxWidth(),shape=RoundedCornerShape(18.dp),color=MaterialTheme.colorScheme.surfaceContainerHigh) {
            Row(Modifier.padding(16.dp),horizontalArrangement=Arrangement.spacedBy(10.dp),verticalAlignment=Alignment.Top) {
                Box(Modifier.padding(top=2.dp)) {Glyph("sticky_note_2",18,"Note")}
                Box(Modifier.weight(1f)) {if(part.rich!=null)RichMessageText(part.rich) else MessageText(part.text,analyze)}
            }
        }
    }
}
