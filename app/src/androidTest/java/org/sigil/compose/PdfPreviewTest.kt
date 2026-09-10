package org.sigil.compose

import android.graphics.Color
import android.graphics.Paint
import android.graphics.pdf.PdfDocument
import android.os.ParcelFileDescriptor
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class PdfPreviewTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    @Test fun isolated_renderer_pages_real_pdf_and_rejects_invalid_content_and_bounds() = runBlocking {
        val context=ui.activity
        val file=java.io.File(context.cacheDir,"synthetic-viewer.pdf")
        val bad=java.io.File(context.cacheDir,"synthetic-invalid.pdf")
        syntheticPdf().also { file.writeBytes(it); it.fill(0) }
        bad.writeText("This is synthetic invalid PDF content.")
        lateinit var session:PdfSession
        ui.runOnIdle { session=PdfSession(context) }
        try {
            for(index in 0..1) {
                val page=session.render(ParcelFileDescriptor.open(file,ParcelFileDescriptor.MODE_READ_ONLY),index,400)
                try { assertEquals(2,page.pages); assertEquals(400,page.bitmap.width); assertEquals(200,page.bitmap.height); assertEquals(if(index==0)Color.RED else Color.BLUE,page.bitmap.getPixel(200,100)) }
                finally { page.bitmap.recycle() }
            }
            val process=InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand("ps -A -o UID,NAME").use { fd ->
                ParcelFileDescriptor.AutoCloseInputStream(fd).bufferedReader().use { it.readText() }
            }.lineSequence().first { it.contains("${context.packageName}:pdf_preview") }
            assertNotEquals(android.os.Process.myUid().toString(),process.trim().substringBefore(' '))
            assertTrue(process.trim().substringBefore(' ').all(Char::isDigit))
            val workerUid=process.trim().substringBefore(' ').toInt()
            assertEquals(android.content.pm.PackageManager.PERMISSION_DENIED,context.checkPermission(android.Manifest.permission.INTERNET,-1,workerUid))
            suspend fun rejected(file:java.io.File,index:Int,width:Int) {
                var rejected=false
                try { session.render(ParcelFileDescriptor.open(file,ParcelFileDescriptor.MODE_READ_ONLY),index,width).bitmap.recycle() }
                catch(_:Exception) { rejected=true }
                assertTrue(rejected)
            }
            rejected(file,2,400)
            rejected(bad,0,400)
            rejected(file,0,100_000)
            val again=session.render(ParcelFileDescriptor.open(file,ParcelFileDescriptor.MODE_READ_ONLY),0,400)
            assertEquals(Color.RED,again.bitmap.getPixel(200,100)); again.bitmap.recycle()
        } finally { session.close(); file.delete(); bad.delete() }
    }
}

internal fun syntheticPdf(): ByteArray {
    val output=java.io.ByteArrayOutputStream()
        val pdf=PdfDocument()
        try {
            for(index in 0..1) {
                val page=pdf.startPage(PdfDocument.PageInfo.Builder(200,100,index+1).create())
                page.canvas.drawRect(0f,0f,200f,100f,Paint().apply { color=if(index==0)Color.RED else Color.BLUE })
                pdf.finishPage(page)
            }
            pdf.writeTo(output)
        } finally { pdf.close() }
    return output.toByteArray()
}
