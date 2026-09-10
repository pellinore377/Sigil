package org.sigil.compose

import android.graphics.Bitmap
import org.json.JSONObject
import java.io.DataInputStream
import java.nio.ByteBuffer

internal sealed interface FilePreview : AutoCloseable {
    override fun close() {}
    data class Text(val text: String,val next: Long?) : FilePreview
    data class Table(val sheets: List<String>,val sheet: Int,val row: Int,val column: Int,val rows: Int,val columns: Int,val cells: List<List<String>>) : FilePreview
    data class Mesh(val bitmap: Bitmap,val triangles: Int) : FilePreview { override fun close() { bitmap.recycle() } }
}

internal fun readFilePreview(stream: DataInputStream,request: String): FilePreview {
    val magic=ByteArray(5);stream.readFully(magic)
    check(magic.contentEquals(byteArrayOf(83,71,80,86,1)))
    val metadata=stream.readInt();val length=stream.readInt()
    check(metadata in 1..1024*1024 && length in 0..16*1024*1024)
    val bytes=ByteArray(metadata)
    val content=try {
        stream.readFully(bytes)
        JSONObject(Charsets.UTF_8.newDecoder().decode(ByteBuffer.wrap(bytes)).toString())
    } finally { bytes.fill(0) }
    val wanted=JSONObject(request)
    return when(content.getString("kind")) {
        "text" -> {
            check(wanted.getString("view")=="text" && length==0)
            val text=content.getString("text");check(text.toByteArray().size<=65536)
            val next=if(content.isNull("next"))null else content.getLong("next")
            check(next==null || next in (wanted.getLong("offset")+1)..128L*1024*1024)
            FilePreview.Text(text,next)
        }
        "table" -> {
            check(wanted.getString("view")=="table" && length==0)
            val names=content.getJSONArray("sheets");check(names.length() in 1..1024)
            val sheets=List(names.length()) { names.getString(it).also { check(it.toByteArray().size<=512) } }
            val sheet=content.getInt("sheet");val row=content.getInt("row");val column=content.getInt("column")
            val rows=content.getInt("total_rows");val columns=content.getInt("total_columns")
            check(sheet==wanted.getInt("sheet") && row==wanted.getInt("row") && column==wanted.getInt("column"))
            check(sheet in sheets.indices && rows in 0..1048576 && columns in 0..16384 && row in 0..rows && column in 0..columns)
            val cells=content.getJSONArray("cells");check(cells.length()<=minOf(128,rows-row))
            val values=List(cells.length()) { r ->
                val cellsInRow=cells.getJSONArray(r);check(cellsInRow.length()<=minOf(32,columns-column))
                List(cellsInRow.length()) { c -> cellsInRow.getString(c).also { check(it.toByteArray().size<=4096) } }
            }
            FilePreview.Table(sheets,sheet,row,column,rows,columns,values)
        }
        "mesh" -> {
            check(wanted.getString("view")=="mesh")
            val width=content.getInt("width");val height=content.getInt("height");val triangles=content.getInt("triangles")
            check(width in 1..minOf(2048,wanted.getInt("width")) && height in 1..2048 && triangles in 1..200000 && length==width*height*4)
            FilePreview.Mesh(readPreviewBitmap(stream,width,height),triangles)
        }
        else -> error("Unsupported file preview")
    }
}

internal fun portableFormat(name: String,mediaType: String): String? = when(name.substringAfterLast('.').lowercase()) {
    "txt","md","json","jsonl","xml","yaml","yml","toml","log","rs","c","cpp","h","html","css" -> "text"
    "csv" -> "csv"
    "tsv" -> "tsv"
    "xls","xlsx","xlsb","ods" -> "spreadsheet"
    "stl" -> "stl"
    "3mf" -> "three_mf"
    "vcf","vcard" -> "contact"
    else -> if(mediaType=="text/plain")"text" else null
}
