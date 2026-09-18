package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.*
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.rememberTextMeasurer

internal val LocalComposerPanelHeight=staticCompositionLocalOf<((Dp)->Unit)?> {null}

@Composable
internal fun toolGridHeight(items:List<Pair<String,String>>,width:Dp):Dp {
    val density=LocalDensity.current
    val measure=rememberTextMeasurer()
    val style=MaterialTheme.typography.labelSmall
    val cell=with(density){((width-32.dp)/5).roundToPx().coerceAtLeast(1)}
    val rows=items.chunked(5)
    return rows.fold(0.dp) {height,row->
        val label=row.maxOf {measure.measure(it.first,style,constraints=Constraints(maxWidth=cell)).size.height}
        height+56.dp+with(density){label.toDp()}
    }+4.dp*(rows.size-1).coerceAtLeast(0)
}

@Composable
internal fun ComposerTool(name:String,icon:String,enabled:Boolean=true,primary:Boolean=false,action:()->Unit) {
    val shape=SigilButtonShape
    Column(Modifier.clip(shape).clickable(enabled=enabled,role=Role.Button,onClick=action).semantics {contentDescription=name}.padding(vertical=2.dp),
        horizontalAlignment=Alignment.CenterHorizontally,verticalArrangement=Arrangement.spacedBy(4.dp)) {
        Surface(Modifier.size(48.dp),shape=shape,color=if(primary)MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
            contentColor=if(primary)MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha=if(enabled)1f else .38f)) {
            Box(contentAlignment=Alignment.Center) {Glyph(icon,24)}
        }
        Text(name,style=MaterialTheme.typography.labelSmall,textAlign=TextAlign.Center,color=MaterialTheme.colorScheme.onSurface.copy(alpha=if(enabled)1f else .38f))
    }
}

@Composable
internal fun AttachmentTools(hasAttachment:Boolean,hasStructured:Boolean,open:(String)->Unit) {
    val features=LocalClientFeatures.current
    val tools=listOf("Photos" to "image","Camera" to "photo_camera","Files" to "draft","One-time location" to "my_location","Real-time location" to "sensors","Drop a pin" to "place","Create" to "add_notes","Format" to "text_format")
    val preferred=LocalComposerPanelHeight.current
    BoxWithConstraints(Modifier.widthIn(max=660.dp).fillMaxWidth()) {
        val height=toolGridHeight(tools,maxWidth-16.dp)+8.dp
        SideEffect {preferred?.invoke(height)}

        LazyVerticalGrid(GridCells.Fixed(5),modifier=Modifier.fillMaxWidth().height(height),contentPadding=PaddingValues(start=8.dp,end=8.dp,top=8.dp),verticalArrangement=Arrangement.spacedBy(4.dp),horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            items(tools,key={it.first}) {(name,icon)->
                val allowed=when(name){"Photos","Files","Camera"->!hasStructured&&features.files;"One-time location","Real-time location","Drop a pin"->!hasStructured&&features.locations;"Format"->true;else->!hasAttachment}
                ComposerTool(name,icon,allowed) {open(name)}
            }
        }
    }
}
