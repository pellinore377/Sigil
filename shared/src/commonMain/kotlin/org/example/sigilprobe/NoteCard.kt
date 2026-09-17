package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

@Composable internal fun NoteCard(part:MessagePart,analyze:(String)->String) {
    val plain=part.rich?.text?:part.text
    val long=plain.length>200 || plain.count {it=='\n'}>=4
    var expanded by remember(part.id){mutableStateOf(false)}
    val measure=with(LocalDensity.current){MaterialTheme.typography.bodyLarge.lineHeight.toDp()*5}
    CardFrame("description","Note") {
        Box(Modifier.heightIn(max=if(long && !expanded)measure else Dp.Unspecified).clipToBounds()) {
            if(part.rich!=null)RichMessageText(part.rich)else MessageText(part.text,analyze)
        }
        if(long)SigilTextButton({expanded=!expanded}) {Glyph(if(expanded)"expand_less"else"expand_more",18);Spacer(Modifier.width(8.dp));Text(if(expanded)"Show less"else"Show more")}
    }
}
