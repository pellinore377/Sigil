package org.sigil.compose

import android.os.ParcelFileDescriptor
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import java.io.*

class FilePreviewTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    @Test fun isolated_native_renderer_pages_literal_text_and_cells_and_rotates_mesh()=runBlocking {
        val file=File(ui.activity.cacheDir,"synthetic-portable")
        lateinit var session:FilePreviewSession
        ui.runOnIdle { session=FilePreviewSession(ui.activity) }
        suspend fun render(format:String,request:String)=session.render(ParcelFileDescriptor.open(file,ParcelFileDescriptor.MODE_READ_ONLY),format,request)
        try {
            file.writeText("x".repeat(65535)+"é<script>literal</script>")
            val first=render("text","""{"view":"text","offset":0}""") as FilePreview.Text
            assertEquals(65535L,first.next);assertEquals(65535,first.text.length)
            val second=render("text","""{"view":"text","offset":65535}""") as FilePreview.Text
            assertEquals("é<script>literal</script>",second.text);assertNull(second.next)
            file.writeText("a,b\n1,\"line1\nline2\"\n=HYPERLINK(1),<script>\n")
            val table=render("csv","""{"view":"table","sheet":0,"row":1,"column":0}""") as FilePreview.Table
            assertEquals(3,table.rows);assertEquals(2,table.columns)
            assertEquals(listOf(listOf("1","line1\nline2"),listOf("=HYPERLINK(1)","<script>")),table.cells)
            java.util.zip.ZipOutputStream(file.outputStream()).use { zip ->
                fun part(name:String,xml:String) { zip.putNextEntry(java.util.zip.ZipEntry(name));zip.write(xml.toByteArray());zip.closeEntry() }
                part("_rels/.rels","""<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>""")
                part("[Content_Types].xml","""<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>""")
                part("xl/workbook.xml","""<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Summary" sheetId="1" r:id="rId1"/><sheet name="Details" sheetId="2" r:id="rId2"/></sheets></workbook>""")
                part("xl/_rels/workbook.xml.rels","""<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>""")
                part("xl/worksheets/sheet1.xml","""<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><f>1+1</f><v>42</v></c></row></sheetData></worksheet>""")
                part("xl/worksheets/sheet2.xml","""<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Second sheet</t></is></c></row></sheetData></worksheet>""")
            }
            val workbook=render("spreadsheet","""{"view":"table","sheet":0,"row":0,"column":0}""") as FilePreview.Table
            assertEquals(listOf("Summary","Details"),workbook.sheets)
            assertEquals(listOf(listOf("42")),workbook.cells)
            val detail=render("spreadsheet","""{"view":"table","sheet":1,"row":0,"column":0}""") as FilePreview.Table
            assertEquals(listOf(listOf("Second sheet")),detail.cells)
            file.writeText("solid example\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid example\n")
            val mesh=render("stl","""{"view":"mesh","yaw":30,"pitch":30,"width":400}""") as FilePreview.Mesh
            try { assertEquals(1,mesh.triangles);assertEquals(400,mesh.bitmap.width) } finally { mesh.close() }
            file.writeText("invalid spreadsheet")
            var rejected=false
            try { render("spreadsheet","""{"view":"table","sheet":0,"row":0,"column":0}""").close() } catch(_:Exception) { rejected=true }
            assertTrue(rejected)
            file.writeText("After rejection")
            assertEquals("After rejection",(render("text","""{"view":"text","offset":0}""") as FilePreview.Text).text)
        } finally { session.close();file.delete() }
    }
    @Test fun main_process_rejects_oversized_or_mismatched_preview_metadata_before_pixels() {
        fun rejected(metadata:String,payload:Int,request:String) {
            val bytes=ByteArrayOutputStream()
            DataOutputStream(bytes).use { out -> out.write(byteArrayOf(83,71,80,86,1));out.writeInt(metadata.toByteArray().size);out.writeInt(payload);out.write(metadata.toByteArray()) }
            var rejected=false
            try { readFilePreview(DataInputStream(ByteArrayInputStream(bytes.toByteArray())),request).close() } catch(_:Exception) { rejected=true }
            assertTrue(rejected)
        }
        rejected("""{"kind":"mesh","width":2147483647,"height":2048,"triangles":1}""",16*1024*1024,"""{"view":"mesh","width":400}""")
        rejected("""{"kind":"text","text":"loop","next":0}""",0,"""{"view":"text","offset":0}""")
        rejected("""{"kind":"table","sheets":["Data"],"sheet":0,"row":128,"column":0,"total_rows":2,"total_columns":2,"cells":[]}""",0,"""{"view":"table","sheet":0,"row":0,"column":0}""")
    }
}
