package org.sigil.compose

import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
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
import kotlinx.coroutines.*
import org.sigil.*

// Pages are rendered as they come into reach and kept two either side; a page that falls out is freed a moment later.
private const val PdfKeep = 2

@Composable
internal fun PdfViewer(message: ChatMessage, close: () -> Unit) {
    val context=LocalContext.current
    val file=message.attachment!!
    var retry by remember { mutableIntStateOf(0) }
    var count by remember(message.id) { mutableIntStateOf(0) }
    val pages=remember(message.id) { mutableStateMapOf<Int,PdfPage>() }
    val zooms=remember(message.id) { mutableStateMapOf<Int,Float>() }
    var failed by remember { mutableStateOf(false) }
    val scope=rememberCoroutineScope()
    val session=remember(message.id,retry) { runCatching { FilePreviewSession(context) }.getOrNull() }
    DisposableEffect(session) { onDispose { session?.close() } }
    val pager=rememberPagerState { count.coerceAtLeast(1) }
    LaunchedEffect(session,pager.currentPage,count) {
        failed=false
        val wanted=listOf(pager.currentPage,pager.currentPage+1,pager.currentPage-1,pager.currentPage+2,pager.currentPage-2).filter { it>=0 && (count==0 || it<count) }.distinct()
        for(index in wanted) {
            if(pages.containsKey(index)) continue
            var produced:PdfPage?=null
            try {
                val result=withContext(Dispatchers.IO) {
                    check(prepare(context,message))
                    checkNotNull(session).render(NativeFileProvider.reader(context,message),index,1600).also { produced=it;check(prepare(context,message)) }
                }
                pages[index]=result;produced=null;count=result.pages
                if(index>=result.pages) break
            } catch(cancelled:CancellationException) { if(cancelled is TimeoutCancellationException) failed=true else throw cancelled }
            catch(_:Exception) { if(index==pager.currentPage) failed=true }
            finally { produced?.bitmap?.recycle() }
        }
        val gone=pages.keys.filter { kotlin.math.abs(it-pager.currentPage)>PdfKeep }
        gone.forEach { index->val old=pages.remove(index);scope.launch { delay(1000);old?.bitmap?.recycle() } }
    }
    DisposableEffect(message.id) { onDispose { val old=pages.values.toList();pages.clear();scope.launch { delay(1000);old.forEach { it.bitmap.recycle() } } } }
    val saver=rememberAttachmentSaver(message)
    Presented(close) {
        DocumentViewerChrome(file.name,"PDF",file.bytes,close,saver.save,saver.saving,caption=file.caption) {
            saver.Notice()
            Column(Modifier.fillMaxSize().padding(horizontal=16.dp,vertical=8.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                Box(Modifier.weight(1f).fillMaxWidth(),contentAlignment=Alignment.Center) {
                    if(failed && pages[pager.currentPage]==null) Column(horizontalAlignment=Alignment.CenterHorizontally) {
                        Text("This PDF could not be displayed.")
                        SigilTextButton({retry++}) { Text("Retry PDF") }
                        SigilTextButton({NativeFileProvider.open(context,message)}) { Text("Open externally") }
                    }
                    // The pages ride a carousel; a zoomed page holds the carousel still and pans instead.
                    else HorizontalPager(pager,Modifier.fillMaxSize(),pageSpacing=16.dp,userScrollEnabled=(zooms[pager.currentPage] ?: 1f)<=1f,beyondViewportPageCount=1) { index->
                        val page=pages[index]
                        if(page==null) Box(Modifier.fillMaxSize(),contentAlignment=Alignment.Center) { CircularProgressIndicator() }
                        else PdfPageView(page,zooms[index] ?: 1f) { zooms[index]=it }
                    }
                }
                if(count>1) Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.SpaceBetween,verticalAlignment=Alignment.CenterVertically) {
                    SigilIconButton({scope.launch { pager.animateScrollToPage(pager.currentPage-1) }},enabled=pager.currentPage>0) { Glyph("chevron_left",24,"Previous PDF page") }
                    Text("Page ${pager.currentPage+1} of $count",Modifier.semantics { liveRegion=LiveRegionMode.Polite },style=MaterialTheme.typography.labelLarge)
                    SigilIconButton({scope.launch { pager.animateScrollToPage(pager.currentPage+1) }},enabled=pager.currentPage+1<count) { Glyph("chevron_right",24,"Next PDF page") }
                }
            }
        }
    }
}

// One page: pinch to zoom, drag to pan while zoomed, double tap to toggle.
@Composable
private fun PdfPageView(page:PdfPage,zoom:Float,onZoom:(Float)->Unit) {
    var pan by remember(page) { mutableStateOf(Offset.Zero) }
    val current by rememberUpdatedState(zoom)
    Box(Modifier.fillMaxSize().graphicsLayer { clip=true }.pointerInput(page) {
        awaitEachGesture {
            awaitFirstDown(requireUnconsumed=false)
            do {
                val event=awaitPointerEvent()
                val scale=event.calculateZoom(); val movement=event.calculatePan()
                if(event.changes.size>1 || current>1f) {
                    val next=(current*scale).coerceIn(1f,5f)
                    val boundX=size.width*(next-1)/2; val boundY=size.height*(next-1)/2
                    pan=Offset((pan.x+movement.x).coerceIn(-boundX,boundX),(pan.y+movement.y).coerceIn(-boundY,boundY))
                    onZoom(next)
                    event.changes.forEach { it.consume() }
                }
            } while(event.changes.any { it.pressed })
        }
    }.pointerInput(page) { detectTapGestures(onDoubleTap={ if(current>1f) { onZoom(1f);pan=Offset.Zero } else onZoom(2f) }) }) {
        Image(page.bitmap.asImageBitmap(),"PDF page ${page.index+1}",Modifier.fillMaxSize().graphicsLayer { scaleX=zoom;scaleY=zoom;translationX=pan.x;translationY=pan.y },contentScale=ContentScale.Fit)
    }
}
