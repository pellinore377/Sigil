package org.sigil

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

@Composable internal fun NoteCard(part:MessagePart,analyze:(String)->String) {
    val plain=part.rich?.text?:part.text
    val long=plain.length>200 || plain.count {it=='\n'}>=4
    var expanded by remember(part.id){mutableStateOf(false)}
    val motion=LocalMotion.current
    val rule=LocalContentColor.current.copy(alpha=.24f)
    val measure=with(LocalDensity.current){MaterialTheme.typography.bodyLarge.lineHeight.toDp()*5}
    Column(Modifier.widthIn(min=200.dp,max=280.dp).animateContentSize(motion.tween(MotionMillis)),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {Glyph("description",20);Text("Note",style=MaterialTheme.typography.labelMedium)}
        Box(Modifier.drawBehind {drawRoundRect(rule,size=Size(2.dp.toPx(),size.height),cornerRadius=CornerRadius(1.dp.toPx()))}.padding(start=14.dp)) {
            Box(Modifier.heightIn(max=if(long && !expanded)measure else Dp.Unspecified).clipToBounds()) {
                if(part.rich!=null)RichMessageText(part.rich)else MessageText(part.text,analyze)
            }
        }
        if(long)SigilTextButton({expanded=!expanded}) {Glyph(if(expanded)"expand_less"else"expand_more",18);Spacer(Modifier.width(8.dp));Text(if(expanded)"Show less"else"Show more")}
    }
}
