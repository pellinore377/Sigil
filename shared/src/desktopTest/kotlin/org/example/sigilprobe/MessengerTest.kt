package org.sigil

import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Rule
import org.junit.Test
import kotlin.test.assertEquals

class MessengerTest {
    @get:Rule val ui = createComposeRule()
    private fun chat(verified: Boolean) = ChatSummary("peer", "@sam:example.com", "", "", verified,
        listOf(ChatDevice("peer", "ab".repeat(32), verified, false, false)))
    @Test fun motion_preferences_follow_account_until_this_device_opts_out() {
        val saved = mutableMapOf("account_appearance" to "Google Sans Flex|Dark|555555|false")
        val commands = mutableListOf<Map<String, Any?>>()
        val systemReduced = mutableStateOf(false)
        var reduced = false
        ui.setContent { androidx.compose.runtime.CompositionLocalProvider(LocalSystemReducedMotion provides systemReduced.value) {
            SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected"), { name, value -> if (name == "organize") commands += value },
                read = saved::get, write = { key, value -> saved[key] = value }, overlay = { val current = LocalMotion.current.reduced; androidx.compose.runtime.SideEffect { reduced = current } })
        } }
        ui.onNodeWithContentDescription("Settings").performClick()
        ui.onAllNodesWithText("Appearance").onLast().performScrollTo().performClick()
        ui.onNodeWithText("Motion & media").performScrollTo().performClick()
        ui.onNodeWithContentDescription("Reduce motion").performClick()
        ui.onNodeWithContentDescription("Message effects").performClick()
        ui.onNodeWithContentDescription("Play GIFs automatically").performClick()
        ui.runOnIdle {
            assertEquals(true, reduced)
            val preference = decodeAppearance(saved["account_appearance"])
            assertEquals("Google Sans Flex", preference.font)
            assertEquals(true, preference.reducedMotion)
            assertEquals(false, preference.messageEffects)
            assertEquals(false, preference.autoplayGifs)
        }
        ui.onNodeWithContentDescription("Back").performClick()
        ui.onNodeWithText("Advanced").performScrollTo().performClick()
        ui.onNodeWithContentDescription("Follow account appearance on this device").performScrollTo().performClick()
        val count = commands.size
        ui.onNodeWithText("Motion & media").performScrollTo().performClick()
        ui.onNodeWithContentDescription("Reduce motion").performClick()
        ui.runOnIdle { assertEquals(false, reduced); assertEquals(count, commands.size); assertEquals(true, decodeAppearance(saved["account_appearance"]).reducedMotion); systemReduced.value = true }
        ui.runOnIdle { assertEquals(true, reduced) }
    }
    @Test fun attachment_caption_stays_editable_and_only_clears_for_its_own_commit() {
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(chat(true)), selected = "peer", transfers = listOf(Transfer("file", "peer", "Photo.jpg", 128, "Ready", true, "image/jpeg"))))
        val sent = mutableListOf<Map<String, Any?>>()
        ui.setContent { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { name, fields -> if (name == "file_send") sent += fields }) }
        ui.onNodeWithTag("composer").performTextInput("Keep this caption")
        ui.onNodeWithContentDescription("Send attachments").performClick()
        ui.runOnIdle { assertEquals("Keep this caption", sent.single()["caption"]); state.value = state.value.copy(issue = "Synthetic send failure") }
        ui.onNodeWithTag("composer").assertTextContains("Keep this caption")
        ui.runOnIdle { state.value = state.value.copy(sent = 1, sentText = "Different send") }
        ui.onNodeWithTag("composer").assertTextContains("Keep this caption")
        ui.onNodeWithTag("composer").performTextReplacement("A new thought")
        ui.runOnIdle { state.value = state.value.copy(sent = 2, sentText = "Keep this caption", transfers = emptyList()) }
        ui.onNodeWithTag("composer").assertTextContains("A new thought")
    }

    @Test fun conversation_settings_are_separate_and_keep_changes_in_the_conversation() {
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        ui.setContent { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat(true)), selected = "peer"), { name, fields -> commands += name to fields }) }
        ui.onNodeWithContentDescription("Conversation menu").performClick()
        ui.onNodeWithText("Settings").performClick()
        ui.onNodeWithText("Conversation settings").assertExists()
        ui.onNodeWithTag("composer").assertDoesNotExist()
        ui.onNodeWithContentDescription("Read receipts").performClick()
        ui.runOnIdle { assertEquals(mapOf("peer" to "peer", "value" to mapOf("ReadReceipts" to false)), commands.last { it.first == "organize" }.second) }
        ui.onNodeWithContentDescription("Back").performClick()
        ui.onNodeWithTag("composer").assertExists()
    }

    @Test fun attachment_forms_keep_unsent_content_until_the_matching_post_succeeds() {
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(chat(true)), selected = "peer"))
        ui.setContent { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { _, _ -> }) }
        fun note() { ui.onNodeWithContentDescription("Create").performClick(); ui.onNodeWithText("Search tools").performTextReplacement("Note"); ui.onNodeWithContentDescription("Note").performClick() }
        ui.onNodeWithContentDescription("Attachments").performClick(); note()
        ui.onNodeWithText("Your note").performTextInput("Keep this thought")
        ui.onNodeWithContentDescription("Back to create").performClick()
        ui.onNodeWithContentDescription("Note").performClick()
        ui.onNodeWithText("Keep this thought").assertExists()
        ui.onNodeWithText("Send", useUnmergedTree = true).performScrollTo().performClick()
        ui.runOnIdle { state.value = state.value.copy(issue = "Synthetic storage failure") }
        ui.onNodeWithText("Keep this thought").assertExists()
        ui.runOnIdle { state.value = state.value.copy(sent = 1, sentText = "Unrelated post", issue = null) }
        ui.onNodeWithText("Keep this thought").assertExists()
        ui.runOnIdle { state.value = state.value.copy(sent = 2, sentText = "note::Keep this thought;") }
        ui.onNodeWithContentDescription("Attachments").performClick(); note()
        ui.onNodeWithText("Keep this thought").assertDoesNotExist()
    }

    @Test fun unaccepted_contact_cannot_send_even_with_a_draft() {
        val commands = mutableListOf<String>()
        ui.setContent { SigilApp(NativeCore::palette, NativeCore::analyze,
            MessengerState(phase = "connected", chats = listOf(chat(false)), selected = "peer"), { name, _ -> commands += name }) }
        ui.onNodeWithTag("composer").performTextInput("hello")
        ui.onNodeWithContentDescription("Send message").assertDoesNotExist()
        ui.onNodeWithText("Send request").assertExists()
        ui.runOnIdle { assertEquals(emptyList(), commands.filter { it in listOf("post", "edit", "card_action") }) }
    }
    @Test fun send_preserves_draft_on_failure_and_clears_after_durable_success() {
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(chat(true)), selected = "peer"))
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        ui.setContent { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { name, fields -> commands += name to fields }) }
        ui.onNodeWithTag("composer").performTextInput("hello")
        ui.onNodeWithContentDescription("Send message").performClick()
        ui.runOnIdle { assertEquals("hello", commands.single { it.first == "post" }.second["text"]); state.value = state.value.copy(issue = "Synthetic storage failure") }
        ui.onNodeWithTag("composer").assertTextContains("hello")
        ui.runOnIdle { state.value = state.value.copy(sent = 1, issue = null) }
        ui.waitForIdle()
        ui.onNodeWithTag("composer").assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.EditableText, androidx.compose.ui.text.AnnotatedString("")))
    }
    @Test fun empty_account_has_no_sample_conversations() {
        ui.setContent { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected"), { _, _ -> }) }
        ui.onNodeWithText("Your correspondence starts here.").assertExists()
        ui.onNodeWithText("Alex Morgan").assertDoesNotExist()
        ui.onNodeWithText("Send locally").assertDoesNotExist()
    }
}

class SignInTest {
    @get:Rule val ui = createComposeRule()
    @Test fun methods_appear_only_after_discovery_and_sso_does_not_request_replacement() {
        val state = mutableStateOf(MessengerState(phase = "new"))
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        ui.setContent { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { name, fields -> commands += name to fields }) }
        ui.onNodeWithText("Server address").assertExists()
        ui.onNodeWithText("Sign in with SSO").assertDoesNotExist()
        ui.onNodeWithText("Username (if registering)").assertDoesNotExist()
        ui.runOnIdle { state.value = state.value.copy(loginAddress = "example.test", loginMethods = LoginMethods("example.test", true, false, false)) }
        ui.onNodeWithText("Sign in with SSO").assertExists().performClick()
        ui.onNodeWithText("Sign in with password").assertDoesNotExist()
        ui.runOnIdle { val fields = commands.first { it.first == "oidc" }.second; assertEquals(false, fields["replace_devices"]); assertEquals(null, fields["username"]) }
        ui.runOnIdle { state.value = state.value.copy(loginAddress = "different.test", loginMethods = null) }
        ui.mainClock.advanceTimeBy(1000)
        ui.onNodeWithText("Sign in with SSO").assertDoesNotExist()
    }
    @Test fun password_fields_are_disclosed_only_when_requested() {
        ui.setContent { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "new", loginAddress = "example.test", loginMethods = LoginMethods("example.test", true, true, false)), { _, _ -> }) }
        ui.onNodeWithText("Password").assertDoesNotExist()
        ui.onNodeWithText("Sign in with password").performClick()
        ui.onNodeWithText("Password").assertExists()
        ui.onNodeWithText("Sign in with SSO").assertExists()
    }
}
