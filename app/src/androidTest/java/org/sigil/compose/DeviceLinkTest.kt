package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*
import org.sigil.storage.NativeStorage
import java.nio.file.Files

class DeviceLinkTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    @Test fun aRealNativeOfferSurvivesReopenAndDecodesOnTheDevice() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val directory = Files.createTempDirectory(context.cacheDir.toPath(), "qr-acceptance").toFile()
        val key = ByteArray(32) { 45 }
        fun run(action: String): JSONObject {
            val result = JSONObject(NativeStorage.execute(directory.absolutePath, key, JSONObject().put("command", "device_link").put("action", action).toString()))
            assertTrue(result.toString(), result.getBoolean("ok")); return result.getJSONObject("value")
        }
        try {
            val offer = run("join")
            assertEquals("show_offer", offer.getString("stage"))
            assertEquals(offer.toString(), run("status").toString())
            val width = offer.getInt("width"); val cells = offer.getString("cells"); val side = (width + 8) * 6
            val pixels = ByteArray(side * side) { 255.toByte() }
            cells.forEachIndexed { index, cell -> if (cell == '1') for (y in 0..5) for (x in 0..5) pixels[((index / width + 4) * 6 + y) * side + (index % width + 4) * 6 + x] = 0 }
            val decoded = NativeStorage.scanLinkQr(side, side, pixels)
            assertTrue(decoded?.startsWith("sigil:link:v1:offer:") == true)
            assertNull(NativeStorage.scanLinkQr(side, side, pixels.copyOf(pixels.size - 1)))
            ui.setContent { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(), { _, _ -> }, overlay = { DeviceLinkDialog(offer, false, null) { _, _ -> } }) }
            ui.onNodeWithContentDescription("Device linking QR code").assertIsDisplayed()
            assertEquals("none", run("cancel").getString("stage"))
        } finally { key.fill(0); directory.deleteRecursively() }
    }
    @Test fun approvalRequiresMatchingSymbolsAndCannotBeDismissedWhileSaving() {
        val flow = JSONObject().put("stage", "confirm_sponsor").put("emoji", org.json.JSONArray(listOf("🐶", "🐱", "🌳", "🍎", "🎂", "🔑", "🚲", "🎸")))
        var confirmed = false
        val busy = androidx.compose.runtime.mutableStateOf(false)
        ui.setContent { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected"), { _, _ -> }, overlay = { DeviceLinkDialog(flow, busy.value, null) { action, _ -> confirmed = action == "confirm"; busy.value = true } }) }
        ui.onNodeWithText("Approve this device").assertIsNotEnabled()
        ui.onNode(isToggleable()).assertIsDisplayed().performClick()
        ui.onNodeWithText("Approve this device").assertIsDisplayed().performClick()
        ui.runOnIdle { assertTrue(confirmed) }
        ui.onNodeWithText("Approve this device").assertIsNotEnabled()
        ui.onNodeWithText("Cancel").assertIsNotEnabled()
    }
}
