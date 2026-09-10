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
    Dialog(close,DialogProperties(usePlatformDefaultWidth=false)) {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                Row(verticalAlignment=Alignment.CenterVertically) {
                    SigilIconButton(close) { Glyph("close",24,"Close file") }
                    Text(file.name,Modifier.weight(1f),style=MaterialTheme.typography.titleMedium,maxLines=2)
                    if(shown is FilePreview.Text) SigilIconButton({clipboard.setText(AnnotatedString(shown.text))},enabled=!loading && !failed) { Glyph("content_copy",24,"Copy visible text") }
                }
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
                    else when(shown) {
                        is FilePreview.Text -> key(offsets.last()) {
                            SelectionContainer(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                                Text(shown.text,Modifier.fillMaxWidth(),fontFamily=LocalCodeFont.current,style=MaterialTheme.typography.bodyMedium)
                            }
                        }
                        is FilePreview.Table -> FileTable(shown) { sheet=it;row=0;column=0 }
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
                    is FilePreview.Table -> {
                        Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.SpaceBetween,verticalAlignment=Alignment.CenterVertically) {
                            SigilIconButton({row=(row-128).coerceAtLeast(0)},enabled=enabled && row>0) { Glyph("expand_less",24,"Previous rows") }
                            Text(if(shown.rows==0)"No rows" else "Rows ${shown.row+1}–${minOf(shown.row+128,shown.rows)} of ${shown.rows}",style=MaterialTheme.typography.labelLarge)
                            SigilIconButton({row+=128},enabled=enabled && row+128<shown.rows) { Glyph("expand_more",24,"Next rows") }
                        }
                        Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.SpaceBetween,verticalAlignment=Alignment.CenterVertically) {
                            SigilIconButton({column=(column-32).coerceAtLeast(0)},enabled=enabled && column>0) { Glyph("chevron_left",24,"Previous columns") }
                            Text(if(shown.columns==0)"No columns" else "Columns ${shown.column+1}–${minOf(shown.column+32,shown.columns)} of ${shown.columns}",style=MaterialTheme.typography.labelLarge)
                            SigilIconButton({column+=32},enabled=enabled && column+32<shown.columns) { Glyph("chevron_right",24,"Next columns") }
                        }
                    }
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
                if(file.caption.isNotBlank()) Box(Modifier.heightIn(max=120.dp).verticalScroll(rememberScrollState())) { MessageText(file.caption,NativeCore::analyze) }
            }
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun FileTable(table: FilePreview.Table,selectSheet: (Int) -> Unit) {
    var selected by remember(table) { mutableStateOf<Pair<String,String>?>(null) }
    val clipboard=LocalClipboardManager.current
    Column(Modifier.fillMaxSize(),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        if(table.sheets.size>1) LazyRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            itemsIndexed(table.sheets) { index,name -> SigilTextButton({selectSheet(index)},enabled=index!=table.sheet) { Text(name,maxLines=1) } }
        }
        val columns=minOf(32,table.columns-table.column)
        if(columns==0 || table.rows==0) Text("This sheet is empty.")
        else Box(Modifier.fillMaxSize().horizontalScroll(rememberScrollState())) {
            LazyColumn(Modifier.width((48+columns*160).dp)) {
                stickyHeader {
                    Row(Modifier.background(MaterialTheme.colorScheme.surfaceContainer)) {
                        Text("#",Modifier.width(48.dp).padding(8.dp),style=MaterialTheme.typography.labelLarge)
                        repeat(columns) { Text("${table.column+it+1}",Modifier.width(160.dp).padding(8.dp),style=MaterialTheme.typography.labelLarge) }
                    }
                }
                itemsIndexed(table.cells) { index,cells ->
                    Row(Modifier.background(MaterialTheme.colorScheme.onSurface.copy(alpha=if(index%2==0).025f else .055f))) {
                        Text("${table.row+index+1}",Modifier.width(48.dp).padding(8.dp),style=MaterialTheme.typography.labelMedium)
                        repeat(columns) { column ->
                            val value=cells.getOrElse(column) { "" }
                            val label="Row ${table.row+index+1}, column ${table.column+column+1}"
                            Text(value,Modifier.width(160.dp).heightIn(min=56.dp).clickable { selected=label to value }.semantics { contentDescription=label }.padding(12.dp),maxLines=4,style=MaterialTheme.typography.bodyMedium)
                        }
                    }
                }
            }
        }
    }
    selected?.let { (label,value) ->
        Dialog({selected=null}) { Surface(shape=MaterialTheme.shapes.large) {
            Column(Modifier.padding(20.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                Text(label,style=MaterialTheme.typography.titleMedium)
                SelectionContainer(Modifier.heightIn(max=360.dp).verticalScroll(rememberScrollState())) { Text(value) }
                Row { SigilTextButton({clipboard.setText(AnnotatedString(value))}) { Text("Copy cell") };SigilTextButton({selected=null}) { Text("Close cell") } }
            }
        } }
    }
}
