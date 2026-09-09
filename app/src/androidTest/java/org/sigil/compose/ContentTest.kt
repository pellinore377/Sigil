package org.sigil.compose

import android.app.Application
import android.graphics.Color
import android.media.AudioManager
import android.os.SystemClock
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.*
import org.json.JSONObject
import org.junit.*
import org.junit.Assert.*
import org.sigil.*
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.sin

class ContentTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext
    @Before fun isolated() { Assume.assumeTrue(context.packageName.endsWith(".acceptance")) }
    private fun native(command: String, fields: Map<String, Any?> = emptyMap()): JSONObject {
        val request = JSONObject().put("command", command); fields.forEach { (k, v) -> request.put(k, JSONObject.wrap(v)) }
        val result = StorageKeyProvider(context).withKey { dir, key -> JSONObject(NativeStorage.execute(dir.path, key, request.toString())) }
        assertTrue(result.toString(), result.getBoolean("ok")); return result.getJSONObject("value")
    }
    @Test fun wallpaperImportsPrivatelyRendersAndCanBeRemoved() = runBlocking {
        val source = java.io.File(context.cacheDir, "synthetic-wallpaper.png")
        val bitmap = android.graphics.Bitmap.createBitmap(800, 600, android.graphics.Bitmap.Config.ARGB_8888)
        bitmap.eraseColor(Color.rgb(25, 115, 185))
        source.outputStream().use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
        bitmap.recycle()
        try { saveWallpaper(context, "self", android.net.Uri.fromFile(source)) } finally { source.delete() }
        val bytes = StorageKeyProvider(context).withKey { directory, key -> NativeStorage.wallpaper(directory.path, key, "self") }
        assertNotNull(bytes); bytes!!.fill(0)
        val revision = androidx.compose.runtime.mutableLongStateOf(0)
        ui.setContent { androidx.compose.material3.Surface(Modifier.fillMaxSize()) { Wallpaper("self", revision.longValue, Modifier.fillMaxSize()) } }
        fun hasBlueImage(): Boolean {
            ui.waitForIdle()
            val screenshot = InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
            try {
                val color = screenshot.getPixel(screenshot.width / 2, screenshot.height / 2)
                return Color.blue(color) - Color.red(color) > 80 && Color.blue(color) - Color.green(color) > 25
            } finally { screenshot.recycle() }
        }
        val deadline = SystemClock.elapsedRealtime() + 8000
        while (!hasBlueImage() && SystemClock.elapsedRealtime() < deadline) delay(100)
        assertTrue("The imported wallpaper was not displayed", hasBlueImage())
        saveWallpaper(context, "self", null)
        ui.runOnIdle { revision.longValue++ }
        val removedDeadline = SystemClock.elapsedRealtime() + 8000
        while (hasBlueImage() && SystemClock.elapsedRealtime() < removedDeadline) delay(100)
        assertFalse("The removed wallpaper is still displayed", hasBlueImage())
        assertNull(StorageKeyProvider(context).withKey { directory, key -> NativeStorage.wallpaper(directory.path, key, "self") })
    }
    @Test fun authenticatedPmtilesRenderTheSyntheticVectorPoint() {
        var failed = false
        ui.setContent { ServerMap(Modifier.fillMaxSize(), failure = { failed = true }) }
        val deadline = SystemClock.elapsedRealtime() + 20000
        var red = 0
        while (SystemClock.elapsedRealtime() < deadline && red < 100) {
            Thread.sleep(250)
            val screenshot = InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
            try {
                red = 0
                for (x in 0 until screenshot.width step 4) for (y in 0 until screenshot.height step 4) {
                    val color = screenshot.getPixel(x, y)
                    if (Color.red(color) in 215..235 && Color.green(color) in 50..75 && Color.blue(color) in 50..75) red++
                }
            } finally { screenshot.recycle() }
        }
        ui.runOnIdle { assertFalse("Map loading failed", failed) }
        assertTrue("The authenticated vector point was not rendered", red >= 100)
    }
    @Test fun encryptedAttachmentPublishesPlaysSeeksAndRejectsReadsAfterDeletion() = runBlocking {
        val state = native("state")
        val chat = state.getJSONArray("chats").getJSONObject(0)
        val peer = chat.getString("id")
        val root = native("timeline", mapOf("peer" to peer)).getJSONArray("messages").getJSONObject(0)
        val pcm = 48000 * 2 * 3
        val bytes = ByteBuffer.allocate(44 + pcm).order(ByteOrder.LITTLE_ENDIAN).apply {
            put("RIFF".toByteArray()); putInt(36 + pcm); put("WAVEfmt ".toByteArray()); putInt(16); putShort(1); putShort(1)
            putInt(48000); putInt(96000); putShort(2); putShort(16); put("data".toByteArray()); putInt(pcm)
            repeat(pcm / 2) { putShort((sin(it * 2.0 * Math.PI * 390 / 48000) * 8000).toInt().toShort()) }
        }.array()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
        val files = NativeFiles(context.applicationContext as Application, scope, { _, _ -> }, { fail(it) })
        try {
            withContext(Dispatchers.IO) { bytes.inputStream().use { files.stage(mapOf("peer" to peer, "reply_author" to root.getString("author"), "reply_message" to root.getString("id")), "Synthetic tone.wav", "audio/wav", bytes.size.toLong(), it) } }
            val deadline = SystemClock.elapsedRealtime() + 60000
            while (SystemClock.elapsedRealtime() < deadline && native("files").getJSONArray("uploads").length() > 0) {
                NativeSync.files(context); NativeSync.run(context, true); delay(300)
            }
            assertEquals(0, native("files").getJSONArray("uploads").length())
            NativeSync.run(context, true)
            var row = native("timeline", mapOf("peer" to peer)).getJSONArray("messages").getJSONObject(0)
            val deliveryDeadline = SystemClock.elapsedRealtime() + 45000
            while (row.getString("delivery") !in listOf("Sent", "Delivered", "Read") && SystemClock.elapsedRealtime() < deliveryDeadline) {
                val sync = NativeSync.run(context, true)
                assertTrue(sync.has("pending"))
                assertTrue(sync.optString("issue"), sync.isNull("issue"))
                delay(300)
                row = native("timeline", mapOf("peer" to peer)).getJSONArray("messages").getJSONObject(0)
            }
            assertTrue("Attachment was queued but never accepted by the server", row.getString("delivery") in listOf("Sent", "Delivered", "Read"))
            val message = ChatMessage(row.getString("id"), row.getString("author"), "", true, "", "Delivered", false, emptyList(), emptyList(), null, true, peer = peer, attachment = AttachmentDetails("Synthetic tone.wav", "audio/wav", bytes.size.toLong()))
            EncryptedMedia(context, message).use { media -> val read = ByteArray(bytes.size); assertEquals(bytes.size, media.readAt(0, read, 0, read.size)); assertArrayEquals(bytes, read); read.fill(0) }
            val audio = context.getSystemService(AudioManager::class.java)
            val volume = audio.getStreamVolume(AudioManager.STREAM_MUSIC)
            audio.setStreamVolume(AudioManager.STREAM_MUSIC, 0, 0)
            try {
                ui.setContent { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected"), { _, _ -> }, overlay = { MediaDialog(message) { } }) }
                ui.waitUntil(10000) { ui.onAllNodesWithText("0:01 / 0:03").fetchSemanticsNodes().isNotEmpty() }
                ui.onNodeWithText("Pause").performClick()
                ui.onNodeWithTag("media-seek").performSemanticsAction(SemanticsActions.SetProgress) { it(2200f) }
                ui.waitUntil(3000) { ui.onAllNodesWithText("0:02 / 0:03").fetchSemanticsNodes().isNotEmpty() }
            } finally { ui.runOnUiThread { ui.activity.setContentView(android.widget.FrameLayout(ui.activity)) }; audio.setStreamVolume(AudioManager.STREAM_MUSIC, volume, 0) }
            native("clear_conversation", mapOf("peer" to peer, "request" to "79".repeat(32), "timestamp" to System.currentTimeMillis() / 1000))
            val chunk = StorageKeyProvider(context).withKey { directory, key -> NativeStorage.readFileChunk(directory.path, key, peer, message.author, message.id, 0) }
            assertNull(chunk)
        } finally { scope.cancel(); bytes.fill(0) }
    }
}
