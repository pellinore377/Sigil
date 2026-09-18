package org.sigil.compose

import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*
import kotlinx.coroutines.*
import org.sigil.*

@Composable
internal fun PdfViewer(message: ChatMessage, close: () -> Unit) {
    val context=LocalContext.current
    val file=message.attachment!!
    var index by remember(message.id) { mutableIntStateOf(0) }
    var retry by remember { mutableIntStateOf(0) }
    var page by remember(message.id) { mutableStateOf<PdfPage?>(null) }
    var failed by remember { mutableStateOf(false) }
    var loading by remember { mutableStateOf(true) }
    var zoom by remember(index) { mutableFloatStateOf(1f) }
    var pan by remember(index) { mutableStateOf(Offset.Zero) }
    val session=remember(message.id,retry) { runCatching { FilePreviewSession(context) }.getOrNull() }
    DisposableEffect(session) { onDispose { session?.close() } }
    LaunchedEffect(session,index) {
        failed=false; loading=true
        var produced: PdfPage? = null
        try {
            val result=withContext(Dispatchers.IO) {
                check(prepare(context,message))
                checkNotNull(session).render(NativeFileProvider.reader(context,message),index,1600).also {
                    produced=it
                    check(prepare(context,message))
                }
            }
            page=result; produced=null
        } catch(cancelled:CancellationException) { if(cancelled is TimeoutCancellationException) failed=true else throw cancelled }
        catch(_:Exception) { failed=true }
        finally { produced?.bitmap?.recycle(); loading=false }
    }
    val shown=page
    // The last frame may still draw the page that was just replaced; it is freed a moment later, not at once.
    val scope=rememberCoroutineScope()
    DisposableEffect(shown) { onDispose { val old=shown; scope.launch { delay(1000); old?.bitmap?.recycle() } } }
    val saver=rememberAttachmentSaver(message)
    Presented(close) {
        DocumentViewerChrome(file.name,"PDF",file.bytes,close,saver.save,saver.saving,caption=file.caption) {
            saver.Notice()
            Column(Modifier.fillMaxSize().padding(horizontal=16.dp,vertical=8.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                Box(Modifier.weight(1f).fillMaxWidth(),contentAlignment=Alignment.Center) {
                    if(loading) CircularProgressIndicator()
                    else if(failed) Column(horizontalAlignment=Alignment.CenterHorizontally) {
                        Text("This PDF could not be displayed.")
                        SigilTextButton({retry++}) { Text("Retry PDF") }
                        SigilTextButton({NativeFileProvider.open(context,message)}) { Text("Open externally") }
                    }
                    else if(shown!=null) Box(Modifier.fillMaxSize().graphicsLayer { clip=true }.pointerInput(index,shown.pages) {
                        // One gesture loop: pinching or a zoomed page pans; at rest a horizontal swipe turns the page on release.
                        awaitEachGesture {
                            awaitFirstDown(requireUnconsumed=false)
                            var travel=0f
                            do {
                                val event=awaitPointerEvent()
                                val scale=event.calculateZoom(); val movement=event.calculatePan()
                                if(event.changes.size>1 || zoom>1f) {
                                    zoom=(zoom*scale).coerceIn(1f,5f)
                                    val boundX=size.width*(zoom-1)/2; val boundY=size.height*(zoom-1)/2
                                    pan=Offset((pan.x+movement.x).coerceIn(-boundX,boundX),(pan.y+movement.y).coerceIn(-boundY,boundY))
                                    event.changes.forEach { it.consume() }
                                } else travel+=movement.x
                            } while(event.changes.any { it.pressed })
                            if(zoom<=1f && kotlin.math.abs(travel)>size.width/5) { if(travel<0 && index+1<shown.pages) index++ else if(travel>0 && index>0) index-- }
                        }
                    }) {
                        Image(shown.bitmap.asImageBitmap(),"PDF page ${shown.index+1}",Modifier.fillMaxSize().graphicsLayer { scaleX=zoom;scaleY=zoom;translationX=pan.x;translationY=pan.y },contentScale=ContentScale.Fit)
                    }

                }
                if(shown!=null && shown.pages>1) Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.SpaceBetween,verticalAlignment=Alignment.CenterVertically) {
                    SigilIconButton({index--},enabled=!loading && index>0) { Glyph("chevron_left",24,"Previous PDF page") }
                    Text(if(shown==null)"PDF" else "Page ${shown.index+1} of ${shown.pages}",Modifier.semantics { liveRegion=LiveRegionMode.Polite },style=MaterialTheme.typography.labelLarge)
                    SigilIconButton({index++},enabled=!loading && shown!=null && index+1<shown.pages) { Glyph("chevron_right",24,"Next PDF page") }
                }
            }
        }
    }
}
