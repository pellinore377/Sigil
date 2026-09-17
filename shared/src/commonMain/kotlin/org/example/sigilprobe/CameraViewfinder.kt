package org.sigil

import androidx.compose.animation.*
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

@Composable
fun CameraViewfinder(modifier:Modifier=Modifier,photo:Boolean,ready:Boolean,busy:Boolean,issue:String?,close:()->Unit,capture:()->Unit,retake:()->Unit,flip:(()->Unit)?=null,flash:Pair<Boolean,()->Unit>?=null,content:@Composable BoxScope.()->Unit) {
    val motionPolicy=LocalMotion.current
    Box(modifier.fillMaxWidth().testTag("camera-viewfinder").clip(RoundedCornerShape(24.dp)).background(Color.Black)) {
        content()
        Box(Modifier.fillMaxWidth().height(88.dp).align(Alignment.TopCenter).background(Brush.verticalGradient(listOf(Color.Black.copy(alpha=.5f),Color.Transparent))))
        Box(Modifier.fillMaxWidth().height(136.dp).align(Alignment.BottomCenter).background(Brush.verticalGradient(listOf(Color.Transparent,Color.Black.copy(alpha=.75f)))))
        CompositionLocalProvider(LocalContentColor provides Color.White) {
            SigilIconButton(close,Modifier.align(Alignment.TopStart).padding(8.dp)){Glyph("close",24,"Back to attachments")}
            AnimatedVisibility(!photo,Modifier.align(Alignment.TopEnd),enter=fadeIn(motionPolicy.enter(MotionMillis)),exit=fadeOut(motionPolicy.exit(MotionExit)),label="Flash control") {
                flash?.let {(active,toggle)->SigilIconButton(toggle,Modifier.padding(8.dp),enabled=!busy){Glyph(if(active)"flash_on" else "flash_off",24,if(active)"Turn flash off" else "Turn flash on")}}
            }
            AnimatedContent(photo,Modifier.align(Alignment.BottomCenter).fillMaxWidth(),
                transitionSpec={fadeIn(motionPolicy.enter(MotionMillis)) togetherWith fadeOut(motionPolicy.exit(MotionExit)) using SizeTransform(clip=false) {_,_->motionPolicy.tween(MotionMillis)}},
                label="Camera controls") {taken->
                Box(Modifier.fillMaxWidth()) {
                    if(taken)SigilIconButton(retake,Modifier.align(Alignment.BottomCenter).padding(bottom=16.dp),enabled=!busy){Glyph("refresh",24,"Retake")}
                    else {
                        IconButton(capture,Modifier.align(Alignment.BottomCenter).padding(bottom=16.dp).size(72.dp).border(3.dp,Color.White.copy(alpha=if(ready)1f else .4f),CircleShape)
                            .semantics {contentDescription=if(busy)"Taking photo" else "Take photo";role=Role.Button},enabled=ready&&!busy) {
                            if(busy)CircularProgressIndicator(Modifier.size(40.dp),color=Color.White,strokeWidth=3.dp)
                            else Box(Modifier.size(56.dp).background(Color.White.copy(alpha=if(ready)1f else .4f),CircleShape))
                        }
                        flip?.let {SigilIconButton(it,Modifier.align(Alignment.BottomEnd).padding(end=8.dp,bottom=28.dp),enabled=!busy){Glyph("flip_camera_android",24,"Switch camera")}}
                    }
                }
            }
            issue?.let {
                Surface(Modifier.align(Alignment.Center).padding(24.dp),shape=RoundedCornerShape(20.dp),color=MaterialTheme.colorScheme.surfaceContainerHigh) {
                    Row(Modifier.padding(horizontal=16.dp,vertical=12.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                        Glyph("error",20)
                        Text(it,Modifier.semantics {liveRegion=LiveRegionMode.Polite},style=MaterialTheme.typography.bodyMedium,maxLines=2,overflow=TextOverflow.Ellipsis)
                    }
                }
            }
        }
    }
}
