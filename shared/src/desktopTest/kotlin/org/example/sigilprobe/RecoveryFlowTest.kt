package org.sigil

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Rule
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class RecoveryFlowTest {
    @get:Rule val ui = createComposeRule()
    private val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
    private fun last(name: String) = commands.last { it.first == name }.second
    private val passkeys = ClientFeatures(passkeys = true)
    private fun app(state: MessengerState, features: ClientFeatures = ClientFeatures(), saved: MutableMap<String, String> = mutableMapOf()) = @Composable {
        CompositionLocalProvider(LocalClientFeatures provides features) {
            SigilApp(NativeCore::palette, NativeCore::analyze, state, { name, fields -> commands += name to fields }, read = saved::get, write = { key, value -> saved[key] = value })
        }
    }

    @Test fun first_screen_links_without_a_server_and_offers_no_recovery() {
        ui.setContent(app(MessengerState(phase = "new")))
        ui.onNodeWithText("Recover a lost account").assertDoesNotExist()
        ui.onNodeWithText("Recover with passkey").assertDoesNotExist()
        ui.onNodeWithText("Link from your phone").assertIsEnabled().performClick()
        ui.runOnIdle { assertEquals(mapOf("action" to "join", "server" to ""), last("device_link")) }
    }

    @Test fun camera_devices_scan_the_other_code_to_join() {
        ui.setContent { CompositionLocalProvider(LocalQrScanner provides { found -> androidx.compose.material3.TextButton({ found("sigil-link:synthetic") }) { Text("Synthetic scan") } }) { app(MessengerState(phase = "new"))() } }
        ui.onNodeWithText("Link from your phone").assertDoesNotExist()
        ui.onNodeWithText("Link from another device").performClick()
        ui.onNodeWithText("Synthetic scan").performClick()
        ui.runOnIdle { assertEquals(mapOf("action" to "join_scan", "qr" to "sigil-link:synthetic"), last("device_link")) }
        ui.onNodeWithText("Server address").assertExists()
    }

    @Test fun sso_waits_quietly_with_cancel_and_reopen() {
        ui.setContent(app(MessengerState(phase = "oidc", loginAddress = "example.test")))
        ui.onNodeWithText("Waiting for your sign-in…").assertExists()
        ui.onNodeWithText("Continue sign-in").assertDoesNotExist()
        ui.onNodeWithText("Open sign-in again").performClick()
        ui.onNodeWithText("Cancel").performClick()
        ui.runOnIdle { assertEquals(listOf("oidc_reopen", "cancel_login"), commands.map { it.first }.filter { it in setOf("oidc_reopen", "cancel_login") }) }
    }

    @Test fun recover_card_sends_passkey_code_link_and_confirmed_reset() {
        ui.setContent(app(MessengerState(phase = "recover", loginAddress = "example.test", recoverAddress = "@sam:example.test", recoverPasskeys = 1), passkeys))
        ui.onNodeWithText("Welcome back").assertExists()
        ui.onNodeWithText("@sam:example.test").assertExists()
        ui.onNodeWithText("Recover with passkey").performClick()
        ui.runOnIdle { assertEquals(emptyMap(), last("passkey_recover")) }
        ui.onNodeWithText("Use a recovery code").performClick()
        ui.onNodeWithText("Recover").assertIsNotEnabled()
        ui.onNodeWithTag("recovery-code").performTextInput("ABCD EFGH-2345 ")
        ui.onNodeWithText("Recover").performClick()
        ui.runOnIdle { assertEquals(mapOf("code" to "ABCDEFGH-2345"), last("recover")) }
        ui.onNodeWithText("Link from your phone instead").performClick()
        ui.runOnIdle { assertEquals("join", last("device_link")["action"]) }
        ui.onNodeWithText("Start over with a new identity").performClick()
        ui.onNodeWithText("Start over with a new identity?").assertExists()
        ui.onNodeWithText("Cancel").performClick()
        ui.runOnIdle { assertTrue(commands.none { it.first == "reset_identity" }) }
        ui.onNodeWithText("Start over with a new identity").performClick()
        ui.onNodeWithText("Start over").performClick()
        ui.runOnIdle { assertEquals(mapOf("confirm" to true), last("reset_identity")) }
    }

    @Test fun recover_card_without_passkeys_asks_for_the_code() {
        ui.setContent(app(MessengerState(phase = "recover", loginAddress = "example.test", recoverAddress = "@sam:example.test", recoverPasskeys = 1)))
        ui.onNodeWithText("Recover with passkey").assertDoesNotExist()
        ui.onNodeWithText("Use a recovery code").assertDoesNotExist()
        ui.onNodeWithTag("recovery-code").assertExists()
    }

    @Test fun passkey_card_follows_a_new_sign_in_once() {
        val saved = mutableMapOf<String, String>()
        val state = mutableStateOf(MessengerState(phase = "new"))
        val generation = mutableIntStateOf(0)
        ui.setContent { key(generation.intValue) { app(state.value, passkeys, saved)() } }
        ui.runOnIdle { state.value = MessengerState(phase = "connected", accountRecovery = AccountRecovery(emptyList(), true)) }
        ui.onNodeWithText("Protect your account").assertExists()
        ui.onNodeWithText("Create passkey").performClick()
        ui.runOnIdle { assertEquals(emptyMap(), last("passkey_create")) }
        ui.onNodeWithText("Not now").performClick()
        ui.onNodeWithText("Before you begin").assertExists()
        ui.runOnIdle { assertEquals("true", saved["passkey_offered"]); state.value = MessengerState(phase = "new"); generation.intValue++ }
        ui.runOnIdle { state.value = MessengerState(phase = "connected", accountRecovery = AccountRecovery(emptyList(), true)) }
        ui.onNodeWithText("Protect your account").assertDoesNotExist()
    }

    @Test fun passkey_card_skips_accounts_that_have_one() {
        val state = mutableStateOf(MessengerState(phase = "new"))
        ui.setContent { app(state.value, passkeys)() }
        ui.runOnIdle { state.value = MessengerState(phase = "connected", accountRecovery = AccountRecovery(listOf(RecoveryPasskey("cred", "Phone", 0)), true)) }
        ui.onNodeWithText("Protect your account").assertDoesNotExist()
    }

    @Test fun devices_offer_showing_or_scanning_a_code() {
        ui.setContent(app(MessengerState(phase = "connected")))
        ui.onNodeWithContentDescription("Settings").performClick()
        ui.onNodeWithText("Devices").performScrollTo().performClick()
        ui.onNodeWithText("Link a new device").performScrollTo().performClick()
        ui.onNodeWithText("Show a code").performClick()
        ui.runOnIdle { assertEquals(mapOf("action" to "sponsor_show"), last("device_link")) }
        ui.onNodeWithText("Link a new device").performScrollTo().performClick()
        ui.onNodeWithText("Scan its code").performClick()
        ui.runOnIdle { assertEquals(mapOf("action" to "sponsor"), last("device_link")) }
    }

    @Test fun sponsor_code_renders_like_an_offer_and_polls() {
        val raw = """{"stage":"show_join","width":21,"cells":"${"10".repeat(220)}1"}"""
        val actions = mutableListOf<String>()
        ui.mainClock.autoAdvance = false
        ui.setContent { MaterialTheme { LinkPanel(raw, false, null, { action, _ -> actions += action }) { } } }
        ui.onNodeWithContentDescription("Device linking QR code").assertExists()
        ui.onNodeWithText("On your new device, choose Link from another device, then scan this code.").assertExists()
        ui.mainClock.advanceTimeBy(1600)
        ui.runOnIdle { assertTrue("poll" in actions) }
    }

    @Test fun account_recovery_settings_list_add_remove_and_reveal() {
        val requests = mutableListOf<String>()
        val state = MessengerState(phase = "connected", accountRecovery = AccountRecovery(listOf(RecoveryPasskey("cred-1", "Laptop", 1_767_225_600)), true),
            storage = StorageDetails(1, 1, 1, 1, recovery = false, checkpoint = null, unprotected = 0))
        ui.setContent { CompositionLocalProvider(LocalServiceAccess provides { raw -> requests += raw; ServiceResponse("""{"code":"ABCD-EFGH-2345"}""") }) { app(state, passkeys)() } }
        ui.onNodeWithContentDescription("Settings").performClick()
        ui.onNodeWithText("Account recovery").performScrollTo().performClick()
        ui.onNodeWithText("Laptop").assertExists()
        ui.onNodeWithText("Added 1 Jan 2026").assertExists()
        ui.onNodeWithText("Remove").performClick()
        ui.onNodeWithText("Remove passkey").performClick()
        ui.runOnIdle { assertEquals(mapOf("credential" to "cred-1"), last("passkey_remove")) }
        ui.onNodeWithText("Add a passkey").performScrollTo().performClick()
        ui.runOnIdle { assertEquals(emptyMap(), last("passkey_create")) }
        ui.onNodeWithText("Show recovery code").performScrollTo().performClick()
        ui.onNodeWithTag("recovery-code-value").assertTextEquals("ABCD-EFGH-2345")
        ui.onNodeWithText("Done").performClick()
        ui.runOnIdle { assertEquals(listOf("""{"command":"recovery_code"}"""), requests) }
        ui.onNodeWithContentDescription("Back").performClick()
        ui.onNodeWithText("Data and storage").performScrollTo().performClick()
        ui.onNodeWithText("Set up recovery").assertDoesNotExist()
        ui.onNodeWithText("Restore with a recovery key").assertDoesNotExist()
    }
}
