package org.sigil.compose

import android.app.Application
import android.graphics.Color
import android.media.AudioManager
import android.os.SystemClock
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
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
    @Test fun reminderBuilderSendsItsConfirmedTimeAndTimezone() = runBlocking {
        lateinit var messenger:Messenger
        val store=androidx.lifecycle.ViewModelStore()
        ui.runOnIdle {messenger=Messenger(context.applicationContext as Application);store.put("reminder",messenger)}
        try {
            ui.waitUntil(10_000) {messenger.state.phase=="connected"}
            ui.runOnIdle {messenger.command("open",mapOf("peer" to "self"))}
            ui.waitUntil(10_000) {messenger.state.selected=="self" && !messenger.state.busy}
            ui.runOnUiThread {ui.activity.setSigilContent {
                CompositionLocalProvider(LocalTemporalPreview provides {kind,input->
                    val result=NativeCore.temporalPreview("$kind\n1772945999\nAmerica/New_York\nmonth\n$input").split('\n')
                    if(result.size==3)TemporalPreview(result[0],"America/New_York","${result[0]} (UTC${result[2]})") else null
                }) {SigilApp(NativeCore::palette,NativeCore::analyze,messenger.state,messenger::command)}
            }}
            ui.onNodeWithContentDescription("Attachments").performClick()
            ui.onNodeWithContentDescription("Create").performClick()
            ui.onNodeWithContentDescription("Reminder").performClick()
            ui.onNodeWithText("Title").performTextInput("Synthetic confirmed reminder")
            ui.onNodeWithText("When").performTextReplacement("not a date")
            ui.onNodeWithText("Send").assertIsNotEnabled()
            ui.onNodeWithText("When").performTextReplacement("07/05/30 9:30am")
            ui.waitUntil(5000) {ui.onAllNodesWithText("2030-07-05T09:30:00 (UTC-04:00)").fetchSemanticsNodes().isNotEmpty()}
            ui.onNodeWithText("Send").performScrollTo().performClick()
            ui.waitUntil(10_000) {messenger.state.sent>0}
            val messages=native("timeline",mapOf("peer" to "self")).getJSONArray("messages")
            val message=(0 until messages.length()).map {messages.getJSONObject(it)}.single {it.getString("text").contains("Synthetic confirmed reminder")}
            val part=message.getJSONArray("parts").getJSONObject(0)
            assertEquals(java.time.Instant.parse("2030-07-05T13:30:00Z").epochSecond,part.getLong("at"))
            assertTrue(message.getString("text").contains("America/New_York"))
        } finally {ui.runOnIdle {store.clear()}}
    }
    @Test fun storedMotionUsesCanonicalRustParametersAndShapedTextUnits() = runBlocking {
        native("post",mapOf("peer" to "self","request" to "d4".repeat(32),"timestamp" to System.currentTimeMillis()/1000,
            "text" to "wave::office العربية 👩🏽‍💻; spoiler::shake::SYNTHETIC_HIDDEN_MOTION;","formatted" to true))
        val messages=native("timeline",mapOf("peer" to "self")).getJSONArray("messages")
        val message=(0 until messages.length()).map {messages.getJSONObject(it)}.single {it.getString("text").startsWith("office العربية")}
        val rich=message.getJSONArray("parts").getJSONObject(0).richText()!!
        val motion=rich.motion.single()
        assertEquals("wave",motion.kind);assertEquals(1200,motion.duration);assertEquals(140,motion.displacement)
        assertEquals(listOf("o","f","f","i","c","e","العربية","👩🏽‍💻"),motion.units.map {rich.text.substring(it.first,it.second)})
        assertFalse(motion.units.any {rich.text.substring(it.first,it.second).contains("SYNTHETIC_HIDDEN_MOTION")})
    }
    @Test fun composerSendsCanonicalFormattingAndRedactsBeforeTheTimeline() = runBlocking {
        lateinit var messenger: Messenger
        val store = androidx.lifecycle.ViewModelStore()
        ui.runOnIdle { messenger = Messenger(context.applicationContext as Application); store.put("composer", messenger) }
        try {
            ui.waitUntil(10_000) { messenger.state.phase == "connected" }
            ui.runOnIdle { messenger.command("open", mapOf("peer" to "self")) }
            ui.waitUntil(10_000) { messenger.state.selected == "self" && !messenger.state.busy }
            ui.runOnUiThread { ui.activity.setSigilContent { SigilApp(NativeCore::palette, NativeCore::analyze, messenger.state, messenger::command) } }
            ui.onNodeWithTag("composer").performClick().performTextInput("underline::Synthetic composer letter; redact::SYNTHETIC_COMPOSER_SECRET;")
            ui.onNodeWithContentDescription("Send message").performClick()
            ui.waitUntil(10_000) { messenger.state.sent > 0 }
            val messages = native("timeline", mapOf("peer" to "self")).getJSONArray("messages")
            val message = (0 until messages.length()).map { messages.getJSONObject(it) }.single { it.getString("text").startsWith("Synthetic composer letter") }
            val rich = message.getJSONArray("parts").getJSONObject(0).getJSONObject("rich")
            assertEquals("Synthetic composer letter [REDACTED]", rich.getString("text"))
            assertTrue(rich.getJSONArray("spans").toString().contains("underline"))
            assertFalse(message.toString().contains("SYNTHETIC_COMPOSER_SECRET"))
            ui.waitUntil(10_000) { messenger.state.messages.any { it.id == message.getString("id") } }
            ui.runOnIdle { messenger.command("edit_source", mapOf("peer" to "self", "author" to message.getString("author"), "message" to message.getString("id"))) }
            ui.waitUntil(10_000) { ui.onAllNodesWithContentDescription("Cancel reply or edit").fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithContentDescription("Cancel reply or edit").assertIsDisplayed()
            ui.onNodeWithTag("composer").performClick()
            val layouts = mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
            ui.onNodeWithTag("composer").performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
            assertEquals("Synthetic composer letter [REDACTED]", layouts.single().layoutInput.text.text)
            assertTrue(layouts.single().layoutInput.text.spanStyles.any { it.item.textDecoration == androidx.compose.ui.text.style.TextDecoration.Underline })
            ui.onNodeWithContentDescription("Send message").performClick()
            ui.waitUntil(10_000) { messenger.state.sent > 1 }
            val edited = native("timeline", mapOf("peer" to "self")).getJSONArray("messages")
            val saved = (0 until edited.length()).map { edited.getJSONObject(it) }.single { it.getString("id") == message.getString("id") }
            assertEquals(rich.toString(), saved.getJSONArray("parts").getJSONObject(0).getJSONObject("rich").toString())
        } finally { ui.runOnIdle { store.clear() } }
    }
    @Test fun locationWorkUsesEncryptedStateAndAnOngoingStopNotification() = runBlocking {
        val now = System.currentTimeMillis() / 1000
        val request = "e7".repeat(32)
        try {
            native("place", mapOf("peer" to "self", "request" to request, "timestamp" to now - 30, "latitude_e6" to 0, "longitude_e6" to 0,
                "accuracy_cm" to 1000, "sampled_at" to now - 30, "label" to "Synthetic live share", "pin" to false, "live" to "fifteen_minutes"))
            val sample = android.location.Location("synthetic").apply { latitude = 1.25; longitude = -2.5; accuracy = 12f; time = now * 1000; elapsedRealtimeNanos = SystemClock.elapsedRealtimeNanos() }
            assertEquals(1, NativeLocations.work(context, sample).getInt("active"))
            val messages = native("timeline", mapOf("peer" to "self")).getJSONArray("messages")
            val message = (0 until messages.length()).map { messages.getJSONObject(it) }.first { it.getString("id") == request }
            assertEquals(1250000, message.getJSONArray("parts").getJSONObject(0).getInt("latitude_e6"))
            NativeLocations.requestStop(context)
            assertEquals(0, NativeLocations.work(context).getInt("active"))
            assertFalse(NativeLocations.stopping(context))
            assertEquals(0, NativeLocations.work(context, sample).getInt("active"))
            InstrumentationRegistry.getInstrumentation().uiAutomation.grantRuntimePermission(context.packageName, android.Manifest.permission.ACCESS_COARSE_LOCATION)
            if (android.os.Build.VERSION.SDK_INT >= 33) InstrumentationRegistry.getInstrumentation().uiAutomation.grantRuntimePermission(context.packageName, android.Manifest.permission.POST_NOTIFICATIONS)
            withContext(Dispatchers.Main) { NativeLocations.prepare(context) }
            assertTrue(context.getSystemService(android.app.NotificationManager::class.java).activeNotifications.any { it.id == 31 })
            withContext(Dispatchers.Main) { NativeLocations.prepared(); context.startService(android.content.Intent(context, LocationService::class.java).setAction("stop")) }
            withTimeout(5000) { while (NativeLocations.running) delay(25) }
            assertFalse(context.getSystemService(android.app.NotificationManager::class.java).activeNotifications.any { it.id == 31 })
        } finally {
            NativeLocations.requestStop(context); NativeLocations.work(context)
            withContext(Dispatchers.Main) { NativeLocations.prepared(); context.stopService(android.content.Intent(context, LocationService::class.java)) }
        }
    }
    @Test fun repeatedSendTapsWhileBusyCreateOneMessage() = runBlocking {
        lateinit var messenger: Messenger
        val store = androidx.lifecycle.ViewModelStore()
        ui.runOnIdle { messenger = Messenger(context.applicationContext as Application); store.put("send", messenger) }
        try {
            ui.waitUntil(10_000) { messenger.state.phase == "connected" }
            val field = Messenger::class.java.getDeclaredField("mutex").apply { isAccessible = true }
            val queue = field.get(messenger) as kotlinx.coroutines.sync.Mutex
            withTimeout(10_000) { queue.lock() }
            try {
                InstrumentationRegistry.getInstrumentation().runOnMainSync {
                    repeat(3) { messenger.command("post", mapOf("peer" to "self", "text" to "Synthetic repeated tap", "rich" to false)) }
                }
            } finally { queue.unlock() }
            withTimeout(10_000) { queue.lock() }
            queue.unlock()
            val messages = native("timeline", mapOf("peer" to "self")).getJSONArray("messages")
            assertEquals(1, (0 until messages.length()).count { messages.getJSONObject(it).getString("text") == "Synthetic repeated tap" })
        } finally { ui.runOnIdle { store.clear() } }
    }
    @Test fun typingAndPostingDoNotWaitForTheNetworkWorker() = runBlocking {
        lateinit var messenger: Messenger
        val store = androidx.lifecycle.ViewModelStore()
        ui.runOnIdle { messenger = Messenger(context.applicationContext as Application); store.put("send", messenger) }
        val field = NativeSync::class.java.getDeclaredField("sync").apply { isAccessible = true }
        val network = field.get(NativeSync) as kotlinx.coroutines.sync.Mutex
        try {
            ui.waitUntil(10_000) { messenger.state.phase == "connected" }
            withTimeout(10_000) { network.lock() }
            try {
                ui.runOnIdle { messenger.foreground(true) }
                delay(1800)
                val peer = messenger.state.chats.first { it.id != "self" && it.verified }.id
                val started = SystemClock.elapsedRealtime()
                InstrumentationRegistry.getInstrumentation().runOnMainSync {
                    messenger.command("typing", mapOf("peer" to peer, "active" to true))
                    messenger.command("post", mapOf("peer" to peer, "text" to "Synthetic concurrent send", "rich" to false))
                }
                withTimeout(3000) { while (!withContext(Dispatchers.Main) { messenger.state.sent == 1L }) delay(10) }
                val timeline = native("timeline", mapOf("peer" to peer)).getJSONArray("messages")
                assertTrue((0 until timeline.length()).any { timeline.getJSONObject(it).getString("text") == "Synthetic concurrent send" })
                assertTrue(network.isLocked)
                InstrumentationRegistry.getInstrumentation().sendStatus(0, android.os.Bundle().apply { putString("stream", "\nSIGIL_LOCAL_SEND_MS=${SystemClock.elapsedRealtime() - started}\n") })
            } finally { ui.runOnIdle { messenger.foreground(false) }; network.unlock() }
        } finally { ui.runOnIdle { store.clear() } }
    }
    @Test fun timelineReadsDoNotWaitForTheNetworkCommandQueue() = runBlocking {
        val text = "Synthetic independent timeline read"
        native("post", mapOf("peer" to "self", "request" to "6b".repeat(32), "timestamp" to System.currentTimeMillis() / 1000, "text" to text))
        lateinit var messenger: Messenger
        val store = androidx.lifecycle.ViewModelStore()
        ui.runOnIdle { messenger = Messenger(context.applicationContext as Application); store.put("timeline", messenger) }
        try {
            ui.waitUntil(10_000) { messenger.state.phase == "connected" && messenger.state.chats.any { it.id != "self" } }
            val field = Messenger::class.java.getDeclaredField("mutex").apply { isAccessible = true }
            val queue = field.get(messenger) as kotlinx.coroutines.sync.Mutex
            withTimeout(10_000) { queue.lock() }
            try {
                var start = 0L
                InstrumentationRegistry.getInstrumentation().runOnMainSync {
                    start = SystemClock.elapsedRealtime(); messenger.command("open", mapOf("peer" to "self"))
                }
                withTimeout(3000) {
                    while (!withContext(Dispatchers.Main) { messenger.state.messages.any { it.text == text } }) delay(5)
                }
                assertTrue("The command queue was released before the local read finished", queue.isLocked)
                InstrumentationRegistry.getInstrumentation().sendStatus(0, android.os.Bundle().apply {
                    putString("stream", "\nSIGIL_LOCAL_TIMELINE_MS=${SystemClock.elapsedRealtime() - start}\n")
                })
            } finally { queue.unlock() }
        } finally { ui.runOnIdle { store.clear() } }
    }
    @Test fun cancellingAnImportStopsItsPreparationAndClearsStaging() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        val started = CompletableDeferred<Unit>()
        val files = NativeFiles(context.applicationContext as Application, scope, { _, _ -> }, { fail(it) })
        try {
            val task = scope.launch { byteArrayOf(1).inputStream().use { stream -> files.stage(mapOf("peer" to "self"), "Synthetic cancellation", "application/octet-stream", 1, stream) { started.complete(Unit); awaitCancellation() } } }
            withTimeout(5000) { started.await() }
            val request = native("files").getJSONArray("uploads").getJSONObject(0).getString("request")
            files.cancel(request)
            withTimeout(5000) { task.join() }
            assertTrue(task.isCancelled)
            assertEquals(0, native("files").getJSONArray("uploads").length())
        } finally { scope.cancel() }
    }
    @Test fun enabledRecoveryPublishesThroughTheAndroidWorker() = runBlocking {
        val secret = native("recovery_generate").getString("secret")
        native("recovery_enable", mapOf("secret" to secret))
        native("post", mapOf("peer" to "self", "request" to "64".repeat(32), "timestamp" to System.currentTimeMillis() / 1000, "text" to "Synthetic backup acceptance"))
        withTimeout(60_000) {
            while (true) {
                val work = NativeSync.files(context)
                assertTrue(work.toString(), work.isNull("issue"))
                val progress = native("storage").getJSONObject("recovery")
                if (!progress.isNull("last") && progress.getLong("pending") == 0L) break
                delay(250)
            }
        }
        java.io.File(context.cacheDir, "acceptance-recovery.key").writeText(secret)
    }
    @Test fun profilePhotoIsBoundedPublishedAndRemoved() = runBlocking {
        val source = java.io.File(context.cacheDir, "synthetic-profile.png")
        val bitmap = android.graphics.Bitmap.createBitmap(1024, 1024, android.graphics.Bitmap.Config.ARGB_8888)
        val random = java.util.Random(7)
        val colors = IntArray(512 * 512) { if (it % 512 in 224..287 && it / 512 in 224..287) Color.rgb(25, 115, 185) else 0xff000000.toInt() or random.nextInt(1 shl 24) }
        bitmap.setPixels(IntArray(1024 * 1024) { colors[(it / 1024 / 2) * 512 + it % 1024 / 2] }, 0, 1024, 0, 0, 1024, 1024)
        source.outputStream().use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
        bitmap.recycle()
        try { stageProfilePhoto(context, android.net.Uri.fromFile(source)) } finally { source.delete() }
        val status = native("photo_status")
        assertTrue(status.getBoolean("pending"))
        val reference = status.getString("avatar")
        val bytes = StorageKeyProvider(context).withKey { directory, key -> NativeStorage.profilePhoto(directory.path, key, reference) }!!
        try {
            assertTrue(bytes.size <= 128 * 1024)
            assertTrue("The fixture must exercise more than one encrypted storage chunk", bytes.size > 65536)
            val bounds = android.graphics.BitmapFactory.Options().apply { inJustDecodeBounds = true }
            android.graphics.BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
            assertEquals(bounds.outWidth, bounds.outHeight)
            assertTrue(bounds.outWidth in 1..512)
        } finally { bytes.fill(0) }
        val published = native("photo_publish")
        assertFalse(published.getBoolean("pending"))
        assertTrue(published.getJSONObject("photo").getInt("bytes") > 0)
        val revision = androidx.compose.runtime.mutableLongStateOf(0)
        ui.setContent { androidx.compose.material3.Surface(Modifier.fillMaxSize()) { ProfilePhoto(reference, revision.longValue, Modifier.fillMaxSize()) } }
        fun blue(): Boolean {
            ui.waitForIdle()
            val screenshot = InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
            try { val color = screenshot.getPixel(screenshot.width / 2, screenshot.height / 2); return Color.blue(color) - Color.red(color) > 80 }
            finally { screenshot.recycle() }
        }
        val deadline = SystemClock.elapsedRealtime() + 8000
        while (!blue() && SystemClock.elapsedRealtime() < deadline) delay(100)
        assertTrue("The published profile photo was not rendered", blue())
        stageProfilePhoto(context, null)
        assertTrue(native("photo_publish").getJSONObject("photo").isNull("hash"))
        ui.runOnIdle { revision.longValue++ }
        val removed = SystemClock.elapsedRealtime() + 8000
        while (blue() && SystemClock.elapsedRealtime() < removed) delay(100)
        assertFalse("The removed profile photo was still rendered", blue())
    }
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
    @Test fun encryptedGifHonorsAutoplayAndReducedMotion() = runBlocking {
        Assume.assumeTrue(android.os.Build.VERSION.SDK_INT >= 28)
        val encoded = java.io.ByteArrayOutputStream()
        encoded.write("GIF89a".toByteArray()); encoded.write(byteArrayOf(16, 0, 16, 0, 0x80.toByte(), 0, 0, -1, 0, 0, 0, 0, -1))
        encoded.write(byteArrayOf(0x21, -1, 11)); encoded.write("NETSCAPE2.0".toByteArray()); encoded.write(byteArrayOf(3, 1, 0, 0, 0))
        for (color in 0..1) {
            encoded.write(byteArrayOf(0x21, 0xf9.toByte(), 4, 0, 10, 0, 0, 0, 0x2c, 0, 0, 0, 0, 16, 0, 16, 0, 0, 2))
            val compressed = java.io.ByteArrayOutputStream()
            var buffer = 0; var bits = 0
            fun code(value: Int) { buffer = buffer or (value shl bits); bits += 3; while (bits >= 8) { compressed.write(buffer and 255); buffer = buffer ushr 8; bits -= 8 } }
            repeat(256) { code(4); code(color) }; code(5)
            if (bits > 0) compressed.write(buffer)
            encoded.write(compressed.size()); encoded.write(compressed.toByteArray()); encoded.write(0)
        }
        encoded.write(0x3b)
        val bytes = encoded.toByteArray()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
        val files = NativeFiles(context.applicationContext as Application, scope, { _, _ -> }, { fail(it) })
        val reduced = androidx.compose.runtime.mutableStateOf(false)
        var request: String? = null
        try {
            withContext(Dispatchers.IO) { bytes.inputStream().use { files.stage(mapOf("peer" to "self", "draft" to true), "Synthetic animation.gif", "image/gif", bytes.size.toLong(), it) } }
            request = native("files").getJSONArray("uploads").getJSONObject(0).getString("request")
            val message = ChatMessage(request, "", "", true, "", "", false, emptyList(), emptyList(), null, true, peer = "self",
                attachment = AttachmentDetails("Synthetic animation.gif", "image/gif", bytes.size.toLong(), "An animated thought.", draft = true))
            ui.setContent { androidx.compose.material3.MaterialTheme { androidx.compose.runtime.CompositionLocalProvider(LocalAppearance provides Appearance(autoplayGifs = false), LocalMotion provides MotionPolicy(reduced.value)) { GifAttachment(message) } } }
            ui.waitUntil(5000) { ui.onAllNodesWithContentDescription("Synthetic animation.gif").fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithContentDescription("Play GIF").assertIsEnabled().performClick()
            fun picture(view: android.view.View): android.widget.ImageView? = if (view is android.widget.ImageView && view.contentDescription == "Synthetic animation.gif") view else if (view is android.view.ViewGroup) (0 until view.childCount).firstNotNullOfOrNull { picture(view.getChildAt(it)) } else null
            val colors = mutableSetOf<Int>()
            val frame = android.graphics.Bitmap.createBitmap(16, 16, android.graphics.Bitmap.Config.ARGB_8888)
            try {
                withTimeout(3000) { while (colors.size < 2) {
                    ui.runOnIdle { val drawable = picture(ui.activity.window.decorView)!!.drawable; drawable.setBounds(0, 0, 16, 16); drawable.draw(android.graphics.Canvas(frame)); colors += frame.getPixel(8, 8) }
                    delay(50)
                } }
            } finally { frame.recycle() }
            assertEquals(setOf(Color.RED, Color.BLUE), colors)
            ui.runOnIdle { reduced.value = true }
            ui.onNodeWithContentDescription("Play GIF").assertIsNotEnabled()
            ui.runOnIdle { assertFalse((picture(ui.activity.window.decorView)!!.drawable as android.graphics.drawable.AnimatedImageDrawable).isRunning) }
            ui.onNodeWithContentDescription("Expand GIF").performClick()
            ui.onNodeWithText("An animated thought.").assertIsDisplayed()
            ui.onNodeWithText("Close").performClick()
            assertTrue(native("files").getJSONArray("uploads").getJSONObject(0).getBoolean("draft"))
        } finally {
            ui.runOnUiThread { ui.activity.setContentView(android.widget.FrameLayout(ui.activity)) }
            request?.let { native("file_cancel", mapOf("request" to it)) }
            scope.cancel(); bytes.fill(0)
        }
    }
    @Test fun encryptedPdfDraftPagesZoomsAndKeepsCaptionWithoutSending() = runBlocking {
        val bytes=syntheticPdf()
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Main)
        val files=NativeFiles(context.applicationContext as Application,scope,{_,_->},{fail(it)})
        var request:String?=null
        try {
            withContext(Dispatchers.IO) { bytes.inputStream().use { files.stage(mapOf("peer" to "self","draft" to true),"Synthetic pages.pdf","application/pdf",bytes.size.toLong(),it) } }
            request=native("files").getJSONArray("uploads").getJSONObject(0).getString("request")
            val message=ChatMessage(request,"","",true,"","",false,emptyList(),emptyList(),null,true,peer="self",attachment=AttachmentDetails("Synthetic pages.pdf","application/pdf",bytes.size.toLong(),"A caption for both pages.",draft=true))
            ui.setContent { androidx.compose.material3.MaterialTheme { AndroidAttachment(message) } }
            ui.onNodeWithText("Download").performClick()
            ui.waitUntil(5000) { ui.onAllNodesWithText("Open").fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithText("Open").performClick()
            ui.waitUntil(15_000) { ui.onAllNodesWithContentDescription("PDF page 1").fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithText("A caption for both pages.").assertIsDisplayed()
            ui.onNodeWithContentDescription("Zoom in PDF").performClick()
            ui.onNodeWithContentDescription("Zoom out PDF").assertIsEnabled().performClick()
            ui.onNodeWithContentDescription("Next PDF page").performClick()
            ui.waitUntil(15_000) { ui.onAllNodesWithContentDescription("PDF page 2").fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithText("Page 2 of 2").assertIsDisplayed()
            ui.onNodeWithContentDescription("Next PDF page").assertIsNotEnabled()
            ui.onNodeWithContentDescription("Close PDF").performClick()
            assertTrue(native("files").getJSONArray("uploads").getJSONObject(0).getBoolean("draft"))
        } finally {
            ui.runOnUiThread { ui.activity.setContentView(android.widget.FrameLayout(ui.activity)) }
            request?.let { native("file_cancel",mapOf("request" to it)) }
            scope.cancel();bytes.fill(0)
        }
    }
    @Test fun encryptedCsvDraftPagesCellsAndKeepsCaptionWithoutSending() = runBlocking {
        val bytes=("Name,Value\n"+(1..130).joinToString("\n") { "Item $it,$it" }).toByteArray()
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Main)
        val files=NativeFiles(context.applicationContext as Application,scope,{_,_->},{fail(it)})
        var request:String?=null
        try {
            withContext(Dispatchers.IO) { bytes.inputStream().use { files.stage(mapOf("peer" to "self","draft" to true),"Synthetic table.csv","text/csv",bytes.size.toLong(),it) } }
            request=native("files").getJSONArray("uploads").getJSONObject(0).getString("request")
            val message=ChatMessage(request,"","",true,"","",false,emptyList(),emptyList(),null,true,peer="self",attachment=AttachmentDetails("Synthetic table.csv","text/csv",bytes.size.toLong(),"The table caption.",draft=true))
            ui.setContent { androidx.compose.material3.MaterialTheme { AndroidAttachment(message) } }
            ui.onNodeWithText("Download").performClick()
            ui.waitUntil(5000) { ui.onAllNodesWithText("Open").fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithText("Open").performClick()
            ui.waitUntil(15_000) { ui.onAllNodesWithText("Rows 1–128 of 131").fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithText("The table caption.").assertIsDisplayed()
            ui.onNodeWithContentDescription("Row 2, column 1").performClick()
            ui.onNodeWithText("Copy cell").performClick()
            assertEquals("Item 1",(context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager).primaryClip!!.getItemAt(0).text.toString())
            ui.onNodeWithText("Close cell").performClick()
            ui.onNodeWithContentDescription("Next rows").performClick()
            ui.waitUntil(15_000) { ui.onAllNodesWithText("Rows 129–131 of 131").fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithText("Item 130").performScrollTo().assertIsDisplayed()
            ui.onNodeWithContentDescription("Next rows").assertIsNotEnabled()
            ui.onNodeWithContentDescription("Previous rows").performClick()
            ui.waitUntil(15_000) { ui.onAllNodesWithText("Rows 1–128 of 131").fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithContentDescription("Close file").performClick()
            assertTrue(native("files").getJSONArray("uploads").getJSONObject(0).getBoolean("draft"))
        } finally {
            ui.runOnUiThread { ui.activity.setContentView(android.widget.FrameLayout(ui.activity)) }
            request?.let { native("file_cancel",mapOf("request" to it)) }
            scope.cancel();bytes.fill(0)
        }
    }
    @Test fun encryptedImageDraftOpensZoomsAndKeepsItsCaptionWithoutSending() = runBlocking {
        val bitmap = android.graphics.Bitmap.createBitmap(2400, 1600, android.graphics.Bitmap.Config.ARGB_8888)
        bitmap.eraseColor(Color.rgb(20, 100, 180))
        val bytes = java.io.ByteArrayOutputStream().use { output -> bitmap.compress(android.graphics.Bitmap.CompressFormat.JPEG, 90, output); output.toByteArray() }
        bitmap.recycle()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
        val files = NativeFiles(context.applicationContext as Application, scope, { _, _ -> }, { fail(it) })
        var request: String? = null
        try {
            withContext(Dispatchers.IO) { bytes.inputStream().use { files.stage(mapOf("peer" to "self", "draft" to true), "Synthetic landscape.jpg", "image/jpeg", bytes.size.toLong(), it) } }
            request = native("files").getJSONArray("uploads").getJSONObject(0).getString("request")
            val message = ChatMessage(request, "", "", true, "", "", false, emptyList(), emptyList(), null, true, peer = "self",
                attachment = AttachmentDetails("Synthetic landscape.jpg", "image/jpeg", bytes.size.toLong(), "The caption stays with the image.", draft = true))
            ui.setContent { androidx.compose.material3.MaterialTheme { AndroidAttachment(message) } }
            ui.waitUntil(5000) { ui.onAllNodesWithContentDescription("Synthetic landscape.jpg").fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithContentDescription("Synthetic landscape.jpg").performClick()
            ui.onNodeWithText("The caption stays with the image.").assertIsDisplayed()
            ui.onNodeWithContentDescription("Zoom in").performClick()
            ui.onNodeWithContentDescription("Zoom out").assertIsEnabled().performClick()
            ui.onNodeWithContentDescription("Fit image").performClick()
            ui.onNodeWithContentDescription("Close image").performClick()
            assertTrue(native("files").getJSONArray("uploads").getJSONObject(0).getBoolean("draft"))
        } finally {
            ui.runOnUiThread { ui.activity.setContentView(android.widget.FrameLayout(ui.activity)) }
            request?.let { native("file_cancel", mapOf("request" to it)) }
            scope.cancel(); bytes.fill(0)
        }
    }
    @Test fun encryptedAttachmentPublishesPlaysSeeksAndRejectsReadsAfterDeletion() = runBlocking {
        val state = native("state")
        val chats = state.getJSONArray("chats")
        val chat = (0 until chats.length()).map { chats.getJSONObject(it) }.single { it.getString("id") != "self" && !it.optBoolean("group") }
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
            withContext(Dispatchers.IO) { bytes.inputStream().use { files.stage(mapOf("peer" to peer, "draft" to true, "reply_author" to root.getString("author"), "reply_message" to root.getString("id")), "Synthetic tone.wav", "audio/wav", bytes.size.toLong(), it) } }
            val upload = native("files").getJSONArray("uploads").getJSONObject(0)
            assertTrue(upload.getBoolean("draft"))
            assertEquals(0, NativeSync.files(context).getInt("sent"))
            assertEquals(root.getString("id"), native("timeline", mapOf("peer" to peer)).getJSONArray("messages").getJSONObject(0).getString("id"))
            val draft = ChatMessage(upload.getString("request"), "", "", true, "", "", false, emptyList(), emptyList(), null, true, peer = peer, attachment = AttachmentDetails("Synthetic tone.wav", "audio/wav", bytes.size.toLong(), draft = true))
            EncryptedMedia(context, draft).use { media -> val read = ByteArray(bytes.size); assertEquals(bytes.size, media.readAt(0, read, 0, read.size)); assertArrayEquals(bytes, read); read.fill(0) }
            val committed = CompletableDeferred<Unit>()
            files.send(upload.getString("request"), "A caption with the recording.") { committed.complete(Unit) }
            withTimeout(10000) { committed.await() }
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
            assertEquals("A caption with the recording.", row.getJSONObject("attachment").getString("caption"))
            val message = ChatMessage(row.getString("id"), row.getString("author"), "", true, "", "Delivered", false, emptyList(), emptyList(), null, true, peer = peer, attachment = AttachmentDetails("Synthetic tone.wav", "audio/wav", bytes.size.toLong(), "A caption with the recording."))
            EncryptedMedia(context, message).use { media -> val read = ByteArray(bytes.size); assertEquals(bytes.size, media.readAt(0, read, 0, read.size)); assertArrayEquals(bytes, read); read.fill(0) }
            val waveform = withContext(Dispatchers.Default) { audioWaveform(context, message, 3000) }
            assertEquals(64, waveform.size)
            assertTrue(waveform.toString(), waveform.all { it in .23f.. .25f })
            val audio = context.getSystemService(AudioManager::class.java)
            val volume = audio.getStreamVolume(AudioManager.STREAM_MUSIC)
            audio.setStreamVolume(AudioManager.STREAM_MUSIC, 0, 0)
            try {
                ui.setContent { androidx.compose.runtime.CompositionLocalProvider(LocalAttachmentContent provides { AndroidAttachment(it) }) {
                    SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", selected = peer,
                        chats = listOf(ChatSummary(peer, "@sam:example.com", "", "", true, emptyList(), displayName = "Sam")), messages = listOf(message)), { _, _ -> })
                } }
                ui.onNodeWithText("A caption with the recording.").assertIsDisplayed()
                ui.onNodeWithContentDescription("Play audio message").performClick()
                ui.waitUntil(10000) { ui.onAllNodesWithText("0:01 / 0:03").fetchSemanticsNodes().isNotEmpty() }
                ui.onNodeWithContentDescription("Pause audio message").performClick()
                ui.onNodeWithTag("audio-seek").performSemanticsAction(SemanticsActions.SetProgress) { it(2200f) }
                ui.waitUntil(3000) { ui.onAllNodesWithText("0:02 / 0:03").fetchSemanticsNodes().isNotEmpty() }
                ui.onNodeWithContentDescription("Expand audio").performClick()
                ui.onNodeWithText("1.5×").performClick()
                ui.onNodeWithText("0:02 / 0:03").assertIsDisplayed()
                ui.onAllNodesWithText("A caption with the recording.").onLast().assertIsDisplayed()
                ui.onNodeWithText("Close").performClick()
                ui.onNodeWithText("0:02 / 0:03").assertIsDisplayed()
            } finally { ui.runOnUiThread { ui.activity.setContentView(android.widget.FrameLayout(ui.activity)) }; audio.setStreamVolume(AudioManager.STREAM_MUSIC, volume, 0) }
            val forward = mapOf("source" to peer, "peer" to "self", "author" to message.author, "message" to message.id)
            val prepared = native("forward", forward + mapOf("request" to "78".repeat(32), "timestamp" to System.currentTimeMillis() / 1000))
            files.forward(forward, prepared.getJSONObject("forward_file"))
            var copy: JSONObject? = null
            withTimeout(60_000) {
                while (copy == null) {
                    NativeSync.files(context)
                    val timeline = native("timeline", mapOf("peer" to "self")).getJSONArray("messages")
                    copy = (0 until timeline.length()).map(timeline::getJSONObject).firstOrNull { !it.isNull("attachment") }
                    delay(250)
                }
            }
            assertEquals("A caption with the recording.", copy!!.getJSONObject("attachment").getString("caption"))
            native("clear_conversation", mapOf("peer" to peer, "request" to "79".repeat(32), "timestamp" to System.currentTimeMillis() / 1000))
            val chunk = StorageKeyProvider(context).withKey { directory, key -> NativeStorage.readFileChunk(directory.path, key, peer, message.author, message.id, 0) }
            assertNull(chunk)
            EncryptedMedia(context, "self", copy!!.getString("author"), copy!!.getString("id"), bytes.size.toLong()).use { media ->
                val read = ByteArray(bytes.size)
                assertEquals(bytes.size, media.readAt(0, read, 0, read.size))
                assertArrayEquals(bytes, read)
                read.fill(0)
            }
        } finally { scope.cancel(); bytes.fill(0) }
    }
}
