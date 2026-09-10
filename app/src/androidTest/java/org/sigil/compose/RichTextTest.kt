package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.text.TextLayoutResult
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class RichTextTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    @Test fun code_expands_copies_exact_whitespace_and_wraps_without_exposing_concealed_code() {
        val source = "let letter = \"👋\";\n" + (1..12).joinToString("\n") { "    // Synthetic line $it with enough content to scroll horizontally without losing indentation" }
        val code = RichText(source, blocks = listOf(RichBlock(0, source.length, "code", language = "rust")), codeTokens = listOf(CodeToken(0, 3, "keyword")))
        val secret = "concealed-code-content"
        val hidden = RichText(secret, listOf(RichSpan(0, secret.length, reveal = "spoiler")), listOf(RichBlock(0, secret.length, "code", language = "rust")))
        val chat = ChatSummary("self", "@sam:example.test", "", "", true, emptyList(), displayName = "Sam")
        fun message(id: String, value: RichText) = ChatMessage(id, "sam", value.text, true, "9:33", "sent", false, emptyList(), emptyList(), null, true, timestamp = 1000, parts = listOf(MessagePart("", "text", value.text, rich = value)))
        ui.runOnUiThread { ui.activity.setSigilContent { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), selected = "self", messages = listOf(message("2", code), message("1", hidden))), { _, _ -> }) } }
        ui.onNodeWithContentDescription("Copy code").performClick()
        val clipboard = ui.activity.getSystemService(android.content.ClipboardManager::class.java)
        ui.runOnIdle { assertEquals(source, clipboard.primaryClip!!.getItemAt(0).text.toString()) }
        ui.onNodeWithText(secret).assertDoesNotExist()
        ui.onNodeWithText("Open code · 13 lines").performClick()
        val node = ui.onNode(hasText(source) and hasAnyAncestor(isDialog()))
        node.assertIsDisplayed()
        fun layout(): TextLayoutResult {
            val layouts = mutableListOf<TextLayoutResult>()
            node.performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
            return layouts.single()
        }
        assertFalse(layout().layoutInput.softWrap)
        ui.onNodeWithText("Wrap lines").performClick()
        assertTrue(layout().layoutInput.softWrap)
        ui.onNodeWithContentDescription("Close code").performClick()
        ui.onNodeWithTag("composer").assertIsDisplayed()
    }
    @Test fun canonical_formatting_links_and_emoji_spoilers_render_on_the_phone() {
        val rich = JSONObject("""{"rich":{"text":"👩🏽‍💻 **literal** link","spans":[{"start":8,"end":19,"effects":[{"kind":"code","value":true}]},{"start":20,"end":24,"effects":[{"kind":"link","value":"https://example.com/letter"}]}],"blocks":[]}}""").richText()!!
        val hidden = RichText("🙈", listOf(RichSpan(0, 2, reveal = "spoiler")))
        val chat = ChatSummary("peer", "@sam:example.com", "", "", true, emptyList(), displayName = "Sam")
        fun message(id: String, text: RichText) = ChatMessage(id, "sam", text.text, false, "9:33am", "", false, emptyList(), emptyList(), null, true, timestamp = 1000, parts = listOf(MessagePart("", "text", text.text, rich = text)))
        var opened: String? = null
        ui.runOnUiThread { ui.activity.setSigilContent {
            CompositionLocalProvider(LocalUriHandler provides object : UriHandler { override fun openUri(uri: String) { opened = uri } }) {
                SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), selected = "peer", messages = listOf(message("2", hidden), message("1", rich))), { _, _ -> })
            }
        } }
        ui.onNodeWithText(rich.text, useUnmergedTree = true).assertIsDisplayed()
        val content = ui.onNodeWithText(rich.text, useUnmergedTree = true).fetchSemanticsNode().config[SemanticsProperties.Text].single()
        assertEquals(rich.text, content.text)
        assertTrue(content.spanStyles.any { it.start == 8 && it.end == 19 && it.item.fontFamily != null })
        ui.onNodeWithText("🙈", useUnmergedTree = true).assertDoesNotExist()
        ui.onNodeWithText("Hidden text", useUnmergedTree = true).assertIsDisplayed().performTouchInput { click(center) }
        ui.waitForIdle()
        ui.onNodeWithText("🙈", useUnmergedTree = true).assertIsDisplayed()
        val layouts = mutableListOf<TextLayoutResult>()
        ui.onNodeWithText(rich.text, useUnmergedTree = true).performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
        val link = layouts.single().getBoundingBox(21).center
        ui.onNodeWithText(rich.text, useUnmergedTree = true).performTouchInput { click(link) }
        ui.runOnIdle { assertEquals("https://example.com/letter", opened) }
        ui.onNodeWithContentDescription("Encrypted message").assertDoesNotExist()
    }
    @Test fun scratching_hidden_text_does_not_trigger_a_message_reply() {
        val rich = RichText("A private letter", listOf(RichSpan(2, 9, reveal = "scratch")))
        val chat = ChatSummary("peer", "@sam:example.com", "", "", true, emptyList(), displayName = "Sam")
        val message = ChatMessage("1", "sam", rich.text, false, "9:33am", "", false, emptyList(), emptyList(), null, true, timestamp = 1000, parts = listOf(MessagePart("", "text", rich.text, rich = rich)))
        ui.runOnUiThread { ui.activity.setSigilContent { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), selected = "peer", messages = listOf(message)), { _, _ -> }) } }
        val node = ui.onNodeWithText("A Scratch to reveal letter", useUnmergedTree = true)
        val layouts = mutableListOf<TextLayoutResult>()
        node.performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
        val start = layouts.single().getBoundingBox(4).center
        val end = layouts.single().getBoundingBox(15).center
        node.performTouchInput { swipe(start, end, 350) }
        ui.onNodeWithText(rich.text, useUnmergedTree = true).assertIsDisplayed()
        ui.onNodeWithContentDescription("Cancel reply or edit").assertDoesNotExist()
    }
}
