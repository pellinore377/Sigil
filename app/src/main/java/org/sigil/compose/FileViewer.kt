package org.sigil.compose

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*
import kotlinx.coroutines.*
import org.json.JSONObject
import org.sigil.*

@Composable
internal fun FileViewer(message: ChatMessage,format: String,close: () -> Unit) {
    val context=LocalContext.current
    val clipboard=LocalClipboardManager.current
    val file=message.attachment!!
    var retry by remember { mutableIntStateOf(0) }
    var page by remember(message.id) { mutableStateOf<FilePreview?>(null) }
    var failed by remember { mutableStateOf(false) }
    var loading by remember { mutableStateOf(true) }
    var offsets by remember(message.id) { mutableStateOf(listOf(0L)) }
    var sheet by remember { mutableIntStateOf(0) }
    var row by remember { mutableIntStateOf(0) }
    var column by remember { mutableIntStateOf(0) }
    var yaw by remember { mutableIntStateOf(0) }
    var pitch by remember { mutableIntStateOf(0) }
    val table=format in listOf("csv","tsv","spreadsheet")
    val delimited=format in listOf("csv","tsv")
    var whole by remember(message.id) { mutableStateOf<List<List<String>>?>(null) }
    LaunchedEffect(message.id,delimited) { if(delimited) whole=runCatching { withContext(Dispatchers.IO) { check(prepare(context,message)); parseDelimited(attachmentHead(context,message,4*1024*1024).decodeToString(),if(format=="tsv")'\t' else ',',8192,256) } }.getOrNull() }
    val mesh=format in listOf("stl","three_mf")
    val request=remember(offsets,sheet,row,column,yaw,pitch) { when {
        table -> JSONObject().put("view","table").put("sheet",sheet).put("row",row).put("column",column)
        mesh -> JSONObject().put("view","mesh").put("yaw",yaw).put("pitch",pitch).put("width",1000)
        else -> JSONObject().put("view","text").put("offset",offsets.last())
    }.toString() }
    val session=remember(message.id,retry) { runCatching { FilePreviewSession(context) }.getOrNull() }
    DisposableEffect(session) { onDispose { session?.close() } }
    LaunchedEffect(session,request) {
        loading=true;failed=false
        var produced:FilePreview?=null
        try {
            val result=withContext(Dispatchers.IO) {
                check(prepare(context,message))
                checkNotNull(session).render(NativeFileProvider.reader(context,message),format,request).also {
                    produced=it;check(prepare(context,message))
                }
            }
            page=result;produced=null
        } catch(cancelled:CancellationException) { if(cancelled is TimeoutCancellationException)failed=true else throw cancelled }
        catch(_:Exception) { failed=true }
        finally { produced?.close();loading=false }
    }
    val shown=page
    DisposableEffect(shown) { onDispose { shown?.close() } }
    val saver=rememberAttachmentSaver(message)
    val kind=attachmentKind(file.name,file.mediaType)
    Dialog(close,DialogProperties(usePlatformDefaultWidth=false,decorFitsSystemWindows=false)) {
        DocumentViewerChrome(file.name,kind.chip,file.bytes,close,saver.save,saver.saving,caption=file.caption) {
            saver.Notice()
            Column(Modifier.fillMaxSize().padding(horizontal=16.dp,vertical=8.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                when(format) {
                    "spreadsheet" -> Text("Cell values · formatting and charts are not shown. Formulas are not recalculated.",style=MaterialTheme.typography.bodySmall)
                    "three_mf" -> Text("Geometry preview · textures and manufacturing details are not shown.",style=MaterialTheme.typography.bodySmall)
                    "contact" -> Text("Contact details · nothing is imported automatically.",style=MaterialTheme.typography.bodySmall)
                }
                Box(Modifier.weight(1f).fillMaxWidth(),contentAlignment=Alignment.Center) {
                    if(loading) CircularProgressIndicator()
                    else if(failed) Column(horizontalAlignment=Alignment.CenterHorizontally) {
                        Text("This file could not be displayed.")
                        SigilTextButton({retry++}) { Text("Retry preview") }
                        SigilTextButton({NativeFileProvider.open(context,message)}) { Text("Open externally") }
                    }
                    else if(delimited && whole!=null) TableDocumentView(whole!!)
                    else when(shown) {
                        is FilePreview.Text -> key(offsets.last()) { TextDocumentView(shown.text,kind==AttachmentKind.Markdown) }
                        is FilePreview.Table -> SheetView(shown,delimited,{sheet=it;row=0;column=0}) {row+=128}
                        is FilePreview.Mesh -> Image(shown.bitmap.asImageBitmap(),"3D geometry preview",Modifier.fillMaxSize(),contentScale=ContentScale.Fit)
                        null -> Unit
                    }
                }
                val enabled=!loading && !failed
                when(shown) {
                    is FilePreview.Text -> Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.SpaceBetween,verticalAlignment=Alignment.CenterVertically) {
                        SigilIconButton({offsets=offsets.dropLast(1)},enabled=enabled && offsets.size>1) { Glyph("chevron_left",24,"Previous text page") }
                        Text("Page ${offsets.size}",style=MaterialTheme.typography.labelLarge)
                        SigilIconButton({shown.next?.let { offsets=offsets+it }},enabled=enabled && shown.next!=null) { Glyph("chevron_right",24,"Next text page") }
                    }
                    is FilePreview.Table -> Unit
                    is FilePreview.Mesh -> {
                        Text("${shown.triangles} triangles · $yaw° / $pitch°",style=MaterialTheme.typography.labelLarge)
                        Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.SpaceEvenly) {
                            SigilIconButton({yaw=(yaw-30)%360},enabled=enabled) { Glyph("rotate_left",24,"Rotate model left") }
                            SigilIconButton({pitch=(pitch-30)%360},enabled=enabled) { Glyph("expand_less",24,"Tilt model up") }
                            SigilIconButton({yaw=0;pitch=0},enabled=enabled) { Glyph("fit_screen",24,"Reset model") }
                            SigilIconButton({pitch=(pitch+30)%360},enabled=enabled) { Glyph("expand_more",24,"Tilt model down") }
                            SigilIconButton({yaw=(yaw+30)%360},enabled=enabled) { Glyph("rotate_right",24,"Rotate model right") }
                        }
                    }
                    null -> Unit
                }
            }
        }
    }
}

// Sheets from the sandbox arrive 128 rows at a time; the grid keeps what it has and asks for more at its foot.
@Composable
private fun SheetView(table: FilePreview.Table,delimited: Boolean,selectSheet: (Int) -> Unit,more: () -> Unit) {
    var rows by remember(table.sheet) { mutableStateOf<List<List<String>>>(emptyList()) }
    LaunchedEffect(table) { rows=rows.take(table.row)+table.cells }
    Column(Modifier.fillMaxSize(),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        if(table.sheets.size>1) LazyRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            itemsIndexed(table.sheets) { index,name -> SigilTextButton({selectSheet(index)},enabled=index!=table.sheet) { Text(name,maxLines=1) } }
        }
        if(table.rows==0 || table.columns==0) Text("This sheet is empty.")
        else TableDocumentView(rows,footer=if(table.row+128<table.rows) ({ SigilTextButton(more) { Text("More rows · ${rows.size} of ${table.rows}") } }) else null)
    }
}
