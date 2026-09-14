package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp

@Composable
internal fun ComposerTool(name:String,icon:String,enabled:Boolean=true,primary:Boolean=false,action:()->Unit) {
    val shape=RoundedCornerShape(22.dp)
    Column(Modifier.clip(RoundedCornerShape(12.dp)).clickable(enabled=enabled,role=Role.Button,onClick=action).semantics {contentDescription=name}.padding(vertical=4.dp),
        horizontalAlignment=Alignment.CenterHorizontally,verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Surface(Modifier.size(64.dp),shape=shape,color=if(primary)MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
            contentColor=if(primary)MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha=if(enabled)1f else .38f)) {
            Box(contentAlignment=Alignment.Center) {Glyph(icon,28)}
        }
        Text(name,style=MaterialTheme.typography.labelLarge,textAlign=TextAlign.Center,color=MaterialTheme.colorScheme.onSurface.copy(alpha=if(enabled)1f else .38f))
    }
}

@Composable
internal fun AttachmentTools(hasAttachment:Boolean,hasStructured:Boolean,open:(String)->Unit) {
    val features=LocalClientFeatures.current
    val tools=listOf("Photos" to "image","Camera" to "photo_camera","Files" to "draft","Place" to "location_on","Contact" to "person",
        "Poll" to "ballot","Checklist" to "checklist","Recipe" to "restaurant","Create" to "add_notes","Format" to "text_format")
    Column(Modifier.fillMaxSize(),horizontalAlignment=Alignment.CenterHorizontally) {
        if(hasStructured || hasAttachment)Text("Remove the current draft to attach a different type.",Modifier.padding(horizontal=20.dp,vertical=8.dp),style=MaterialTheme.typography.bodySmall)
        LazyVerticalGrid(if(LocalWideLayout.current)GridCells.Fixed(5) else GridCells.Adaptive(64.dp),modifier=Modifier.widthIn(max=600.dp).fillMaxWidth(),contentPadding=PaddingValues(horizontal=16.dp,vertical=16.dp),verticalArrangement=Arrangement.spacedBy(16.dp),horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            items(tools,key={it.first}) {(name,icon)->
                val allowed=when(name){"Photos","Files","Camera"->!hasStructured&&features.files;"Place"->!hasStructured&&features.locations;"Format"->true;else->!hasAttachment}
                ComposerTool(name,icon,allowed) {open(name)}
            }
        }
    }
}
