package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.activity.enableEdgeToEdge
import android.view.WindowManager
import org.junit.Before
import android.graphics.Bitmap
import java.io.File
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Rule
import org.junit.Test
import org.junit.Assert.*
import org.sigil.*

class MessagingUiTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    private val chat = ChatSummary("peer", "@sam:example.com", "A little correspondence", "9:33am", true, emptyList(), displayName = "Sam", unread = 2, pinned = true)
    private fun message(id: String, mine: Boolean) = ChatMessage(id, if (mine) "me" else "sam", if (mine) "See you tomorrow." else "A little correspondence", mine, "9:33am", if (mine) "Delivered" else "", false, emptyList(), emptyList(), null, true, timestamp = 1000, separator = "Today, 9:33am")
    @Before fun windowInsetsMatchTheApplication() {
        androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().runOnMainSync {
            ui.activity.enableEdgeToEdge()
            ui.activity.window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)
        }
    }
    private fun show(content: @androidx.compose.runtime.Composable () -> Unit) { ui.runOnUiThread { ui.activity.setSigilContent(content) }; ui.waitForIdle() }
    private fun screenshot(name: String) {
        ui.waitForIdle()
        val bitmap = androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
        File(ui.activity.cacheDir, "ui-$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        bitmap.recycle()
    }
    @Test fun firstContactRequestKeepsTheDraftAndNeverSendsBeforeVerification() {
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(chat.copy(verified = false)), selected = "peer"))
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        show { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { name, fields -> commands += name to fields }) }
        ui.onNodeWithText("Sam").assertIsDisplayed()
        ui.onNodeWithTag("composer").performTextInput("A synthetic first letter")
        ui.onNodeWithContentDescription("Send request").performClick()
        ui.runOnIdle {
            assertTrue(commands.any { it.first == "contact_request" && it.second["action"] == "send" })
            assertFalse(commands.any { it.first == "post" || it.first == "confirm" || it.first == "typing" })
            state.value = state.value.copy(chats = listOf(chat.copy(verified = false, request = "pending")))
        }
        ui.onNodeWithTag("composer").assertTextContains("A synthetic first letter")
        ui.onNodeWithContentDescription("Send message").assertIsNotEnabled()
        ui.waitUntil(5000) { ui.onNodeWithText("Sam").isDisplayed() }
        ui.onNodeWithText("Sam").assertIsDisplayed()
        ui.onNodeWithText("Request sent. Waiting for Sam to accept and verify your device.").assertIsDisplayed()
        ui.mainClock.advanceTimeBy(500)
        ui.waitForIdle()
        Thread.sleep(250)
        screenshot("request-pending")
    }
    @Test fun acceptingARequestDoesNotApproveItsEncryptionIdentity() {
        val contact = chat.copy(verified = false, request = "incoming", devices = listOf(ChatDevice("device", "0123".repeat(16), false, false, false)))
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(contact), selected = "peer"))
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        show { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { name, fields -> commands += name to fields }) }
        ui.onNodeWithText("Verify devices").assertDoesNotExist()
        screenshot("request-incoming")
        ui.onNodeWithText("Accept").performClick()
        ui.runOnIdle {
            assertTrue(commands.any { it.first == "contact_request" && it.second["action"] == "accept" })
            assertFalse(commands.any { it.first == "confirm" })
            state.value = state.value.copy(chats = listOf(contact.copy(request = "accepted")))
        }
        ui.onNodeWithText("Verify devices").performClick()
        ui.onNodeWithText("Fingerprints match · approve").performClick()
        ui.runOnIdle { assertTrue(commands.any { it.first == "confirm" && it.second["peer"] == "device" }) }
    }
    @Test fun recoveryRequiresASavedKeyAndProtectsItsWindow() {
        var enabled = false
        val secret = "abcde012".repeat(8)
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected"), { _, _ -> }, overlay = { RecoveryDialog(secret, false, {}) { enabled = true } }) }
        ui.onNodeWithText("Enable encrypted backups").assertIsNotEnabled()
        ui.onNode(isToggleable()).performScrollTo().performClick()
        ui.onNodeWithText("Last 8 characters of your saved key").performScrollTo().performTextInput("00000000")
        ui.onNodeWithText("Enable encrypted backups").assertIsNotEnabled()
        ui.onNodeWithText("Last 8 characters of your saved key").performTextReplacement(secret.takeLast(8))
        if (android.os.Build.VERSION.SDK_INT >= 29) ui.runOnIdle {
            assertTrue(android.view.inspector.WindowInspector.getGlobalWindowViews().any { view ->
                ((view.layoutParams as? WindowManager.LayoutParams)?.flags ?: 0) and WindowManager.LayoutParams.FLAG_SECURE != 0
            })
        }
        ui.onNodeWithText("Enable encrypted backups").performClick()
        ui.runOnIdle { assertTrue(enabled) }
    }
    @Test fun signOutRequiresAcknowledgingLocalLossAndDoesNotAssumeRevocation() {
        val stage = mutableStateOf("confirm")
        val commands = mutableListOf<String>()
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected"), { _, _ -> }, overlay = { SignOutDialog(stage.value, false, null) { commands += it } }) }
        ui.onNodeWithText("Sign out").assertIsNotEnabled()
        ui.onNodeWithText("Cancel").performClick()
        ui.runOnIdle { assertEquals(listOf("cancel"), commands) }
        ui.onNode(isToggleable()).performScrollTo().performClick()
        ui.onNodeWithText("Sign out").performClick()
        ui.runOnIdle { assertEquals(listOf("cancel", "confirm"), commands); stage.value = "pending" }
        ui.onNodeWithText("Cancel").assertDoesNotExist()
        ui.onNodeWithText("Remove local data").assertDoesNotExist()
        ui.onNodeWithText("Retry revocation").performClick()
        ui.runOnIdle { assertEquals("retry", commands.last()) }
        ui.onNode(isToggleable()).performScrollTo().performClick()
        ui.onNodeWithText("Remove local data").performClick()
        ui.runOnIdle { assertEquals("erase", commands.last()) }
    }
    @Test fun signInChangesRequireReviewOfTheCurrentConfiguration() {
        val access = AccountAccess(3, 8, "https://identity.example", true, true, false, false)
        val state = mutableStateOf(MessengerState(phase = "connected", address = "@sam:example.com", profileName = "Sam", profileRevision = 0, accountAccess = access))
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        show { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { name, fields -> commands += name to fields }) }
        ui.onNodeWithText("Your server’s sign-in is changing · Review").performClick()
        ui.onNodeWithText("Acknowledge sign-in change").performScrollTo().assertIsNotEnabled()
        ui.onNode(isToggleable()).performScrollTo().performClick()
        ui.runOnIdle { state.value = state.value.copy(accountAccess = access.copy(transition = 9)) }
        ui.onNodeWithText("Acknowledge sign-in change").performScrollTo().assertIsNotEnabled()
        ui.onNode(isToggleable()).performScrollTo().performClick()
        ui.onNodeWithText("Acknowledge sign-in change").performScrollTo().performClick()
        ui.runOnIdle {
            assertEquals(listOf("acknowledge_access" to mapOf("configuration_revision" to 3L, "transition_revision" to 9L)), commands.filter { it.first == "acknowledge_access" })
            state.value = state.value.copy(accountAccess = access.copy(linked = false, retiring = false))
        }
        ui.onNodeWithText("Link SSO account").performScrollTo().performClick()
        ui.runOnIdle {
            assertTrue(commands.any { it.first == "oidc_account" && it.second["action"] == "start" })
            assertFalse(commands.any { it.first == "oidc" || it.first == "enroll" })
        }
    }
    @Test fun historyRestoreRequiresTheKeyAndExplicitBackupReview() {
        var received: String? = null
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected"), { _, _ -> }, overlay = { RestoreRecoveryDialog(false, null, {}) { received = it } }) }
        ui.onNodeWithText("Restore history").assertIsNotEnabled()
        ui.onNodeWithText("Recovery key").performScrollTo().performTextInput("ABCD ".repeat(16))
        ui.onNodeWithText("Restore history").assertIsNotEnabled()
        ui.onNode(isToggleable()).performScrollTo().performClick()
        ui.onNodeWithText("Restore history").performClick()
        ui.runOnIdle { assertEquals("abcd".repeat(16), received) }
    }
    @Test fun savedHistoryDoesNotOfferLiveMessagingOrCallControls() {
        val archived = chat.copy(id = "history:synthetic", displayName = "Saved conversation", archived = true)
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(archived), selected = archived.id, messages = listOf(message("out", true))), { _, _ -> }) }
        ui.onNodeWithText("Saved conversation").assertIsDisplayed()
        ui.onNodeWithTag("composer").assertDoesNotExist()
        ui.onNodeWithContentDescription("Start audio call").assertDoesNotExist()
        ui.onNodeWithContentDescription("Restored encrypted message").assertDoesNotExist()
        ui.onNodeWithText("See you tomorrow.").performClick()
        ui.onNodeWithContentDescription("Restored encrypted message").assertIsDisplayed()
        screenshot("saved-history")
    }
    @Test fun switchingComposerPanelsKeepsTheComposerSteady() {
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), selected = "peer", messages = listOf(message("out", true))), { _, _ -> }) }
        val initial = ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInRoot.top
        ui.onNodeWithTag("composer").performClick()
        try { ui.waitUntil(5000) { ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInRoot.top < initial - 100 } } finally { screenshot("keyboard") }
        ui.waitUntil(5000) { ui.onNodeWithText("Sam").isDisplayed() }
        val keyboard = ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInRoot.top
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithText("Photos").assertIsDisplayed()
        assertEquals(keyboard, ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInRoot.top, 3f)
        screenshot("attachments")
        ui.onNodeWithContentDescription("Voice message").performClick()
        ui.onNodeWithText("Record").assertIsDisplayed()
        assertEquals(keyboard, ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInRoot.top, 3f)
        ui.onNodeWithContentDescription("Show keyboard").performClick()
        repeat(8) {
            ui.mainClock.advanceTimeBy(32)
            assertEquals(keyboard, ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInRoot.top, 3f)
        }
    }
    @Test fun messageDetailsStartHiddenAndReceiptsFollowTheLastTimelineMessage() {
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(chat), selected = "peer", messages = listOf(message("out", true), message("in", false))))
        show { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { _, _ -> }) }
        ui.onNodeWithContentDescription("Delivered").assertExists()
        ui.onNodeWithContentDescription("Encrypted message").assertDoesNotExist()
        ui.onNodeWithText("See you tomorrow.").performClick()
        ui.onNodeWithContentDescription("Encrypted message").assertExists()
        screenshot("timeline")
        ui.runOnIdle { state.value = state.value.copy(messages = listOf(message("new-in", false)) + state.value.messages) }
        ui.onNodeWithContentDescription("Delivered").assertDoesNotExist()
    }
    @Test fun longPressOpensTheReactionBubbleActionSandwich() {
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), selected = "peer", messages = listOf(message("out", true))), { _, _ -> }) }
        ui.onNodeWithText("See you tomorrow.").performTouchInput { longClick() }
        ui.onNodeWithText("Reply in thread").assertIsDisplayed()
        ui.onNodeWithText("Copy").assertIsDisplayed()
        ui.onNodeWithContentDescription("Choose reaction").assertIsDisplayed()
        screenshot("message-menu")
    }
    @Test fun searchOffersTheEightCategoriesAndNotesExcludesEmptyConversations() {
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat)), { _, _ -> }) }
        screenshot("inbox")
        ui.onNodeWithContentDescription("Search conversations").performClick()
        listOf("Unread", "Conversations", "Requests", "Pinned", "Images", "Videos", "Places", "Links").forEach { ui.onNodeWithText(it).assertExists() }
        ui.onNodeWithContentDescription("Back").performClick()
        ui.onNodeWithContentDescription("Conversation notes").performClick()
        ui.onNodeWithText("Sam").assertDoesNotExist()
        ui.onNodeWithText("Your conversation notes will appear here.").assertExists()
    }
    @Test fun emojiOnlyMessagesUseTheBundledAnimatedArtwork() {
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), selected = "peer", messages = listOf(message("emoji", true).copy(text = "😀"))), { _, _ -> }) }
        ui.waitUntil(10000) { ui.onAllNodesWithTag("animated-emoji:1f600").fetchSemanticsNodes().isNotEmpty() }
        screenshot("emoji")
    }
}
