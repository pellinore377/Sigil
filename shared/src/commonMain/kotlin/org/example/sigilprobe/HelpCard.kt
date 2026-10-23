@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
package org.sigil

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

// Inserts SigilText at the composer's cursor; absent where there is no composer to try it in.
internal val LocalComposerInsert=staticCompositionLocalOf<((String)->Unit)?> {null}
private val HelpBleed=8.dp

// A shared cheat sheet: the title, a lead, then each construct's name over an example that inserts into the composer.
@Composable internal fun HelpCard(value:UtilityContent) {
    val ink=LocalContentColor.current
    val insert=LocalComposerInsert.current
    Column(Modifier.widthIn(min=MessageCardMinWidth,max=MessageCardMaxWidth).fillMaxWidth().padding(vertical=4.dp)) {
        // The type word rides on the title alone, so the title is announced once.
        Text(value.display,Modifier.semantics {heading();contentDescription="SigilText help. ${value.display}"},style=MaterialTheme.typography.titleMedium,maxLines=3,overflow=TextOverflow.Ellipsis)
        value.secondary?.let {Text(it.text,Modifier.padding(top=4.dp),style=MaterialTheme.typography.labelMedium,color=ink.copy(alpha=.68f))}
        if(value.entries.isNotEmpty())Column(Modifier.padding(top=8.dp)) {
            value.entries.forEach {HelpRow(it,insert)}
        }
        if(value.alternate.isNotEmpty())Text(value.alternate,Modifier.padding(top=8.dp),style=MaterialTheme.typography.labelMedium,color=ink.copy(alpha=.68f))
    }
}

@Composable private fun HelpRow(entry:HelpEntry,insert:((String)->Unit)?) {
    val motion=LocalMotion.current
    val ink=LocalContentColor.current
    val interaction=remember {MutableInteractionSource()}
    val pressed by interaction.collectIsPressedAsState()
    val hovered by interaction.collectIsHoveredAsState()
    val focused by interaction.collectIsFocusedAsState()
    val fill by animateFloatAsState(if(insert==null)0f else if(focused && !pressed).10f else if(pressed || hovered).07f else 0f,motion.tween(MotionFeedback),label="Help row")
    val tap=if(insert==null)Modifier else Modifier.combinedClickable(interaction,null,role=Role.Button,onClickLabel="Try it in the composer",onLongClick=LocalMaterialPress.current) {insert(entry.code)}
    Column(Modifier.widen(HelpBleed).fillMaxWidth().heightIn(min=48.dp).clip(RoundedCornerShape(14.dp)).background(ink.copy(alpha=fill)).then(tap)
        .semantics(mergeDescendants=true) {}.padding(horizontal=HelpBleed,vertical=8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
        Text(entry.label,style=MaterialTheme.typography.bodyMedium,maxLines=2,overflow=TextOverflow.Ellipsis)
        Text(entry.code,style=MaterialTheme.typography.labelMedium.copy(fontFamily=LocalCodeFont.current),color=ink.copy(alpha=.68f),maxLines=4,overflow=TextOverflow.Ellipsis)
    }
}
