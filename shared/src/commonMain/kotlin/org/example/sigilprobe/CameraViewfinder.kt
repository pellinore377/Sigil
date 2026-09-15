package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

@Composable
fun CameraViewfinder(modifier:Modifier=Modifier,photo:Boolean,ready:Boolean,busy:Boolean,issue:String?,close:()->Unit,capture:()->Unit,retake:()->Unit,flip:(()->Unit)?=null,flash:Pair<Boolean,()->Unit>?=null,content:@Composable BoxScope.()->Unit) {
    Box(modifier.fillMaxWidth().testTag("camera-viewfinder").clip(RoundedCornerShape(24.dp)).background(Color.Black)) {
        content()
        Box(Modifier.fillMaxWidth().height(88.dp).align(Alignment.TopCenter).background(Brush.verticalGradient(listOf(Color.Black.copy(alpha=.5f),Color.Transparent))))
        Box(Modifier.fillMaxWidth().height(136.dp).align(Alignment.BottomCenter).background(Brush.verticalGradient(listOf(Color.Transparent,Color.Black.copy(alpha=.75f)))))
        CompositionLocalProvider(LocalContentColor provides Color.White) {
            SigilIconButton(close,Modifier.align(Alignment.TopStart).padding(8.dp)){Glyph("close",28,"Back to attachments")}
            if(!photo)flash?.let {(active,toggle)->SigilIconButton(toggle,Modifier.align(Alignment.TopEnd).padding(8.dp),enabled=!busy){Glyph(if(active)"flash_on" else "flash_off",26,if(active)"Turn flash off" else "Turn flash on")}}
            if(photo)SigilIconButton(retake,Modifier.align(Alignment.BottomCenter).padding(bottom=20.dp),enabled=!busy){Glyph("refresh",28,"Retake")}
            else {
                IconButton(capture,Modifier.align(Alignment.BottomCenter).padding(bottom=16.dp).size(72.dp).border(3.dp,Color.White.copy(alpha=if(ready)1f else .4f),CircleShape),enabled=ready&&!busy) {
                    if(busy)CircularProgressIndicator(Modifier.size(40.dp),color=Color.White,strokeWidth=3.dp)
                    else Box(Modifier.size(56.dp).background(Color.White.copy(alpha=if(ready)1f else .4f),CircleShape).semantics {contentDescription="Take photo"})
                }
                flip?.let {SigilIconButton(it,Modifier.align(Alignment.BottomEnd).padding(end=16.dp,bottom=28.dp),enabled=!busy){Glyph("flip_camera_android",28,"Switch camera")}}
            }
            issue?.let {Text(it,Modifier.align(Alignment.Center).padding(24.dp).background(Color.Black.copy(alpha=.7f),RoundedCornerShape(12.dp)).padding(12.dp),style=MaterialTheme.typography.bodySmall)}
        }
    }
}
