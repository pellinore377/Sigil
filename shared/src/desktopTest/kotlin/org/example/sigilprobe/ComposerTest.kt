@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class, androidx.compose.ui.test.ExperimentalTestApi::class)

package org.sigil

import androidx.compose.foundation.text.input.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.Clipboard
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Rule
import org.junit.Test
import kotlin.test.assertEquals
import java.awt.datatransfer.DataFlavor
import java.awt.datatransfer.Transferable

class ComposerTest {
    @get:Rule val ui = createComposeRule()
    private lateinit var state: TextFieldState
    private val clipboard = object : Clipboard {
        var entry: ClipEntry? = null
        override val nativeClipboard = java.awt.datatransfer.Clipboard("synthetic composer test")
        override suspend fun getClipEntry() = entry
        override suspend fun setClipEntry(clipEntry: ClipEntry?) { entry = clipEntry }
    }
    private val field get() = ui.onNodeWithTag("composer")

    private fun open(source: String) {
        state = TextFieldState(source)
        ui.setContent {
            CompositionLocalProvider(LocalClipboard provides clipboard) {
                MaterialTheme { Composer(state, NativeCore::analyze) }
            }
        }
        field.performClick()
    }
    private fun rendered(): String {
        val results = mutableListOf<TextLayoutResult>()
        field.performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(results) }
        return results.single().layoutInput.text.text
    }

    @Test fun hidden_syntax_round_trips_without_changing_source() {
        val source = "**bold *inner*** and `**code**` 👩🏽‍💻 שלום"
        open(source)
        assertEquals("bold inner and **code** 👩🏽‍💻 שלום", rendered())
        ui.onNodeWithText("Source").performClick()
        assertEquals(source, rendered())
        ui.onNodeWithText("Formatted").performClick()
        ui.runOnIdle { assertEquals(source, state.text.toString()) }
    }

    @Test fun backspace_at_hidden_closing_delimiter_edits_content() {
        open("**bold** tail")
        field.performTextInputSelection(TextRange(4), relativeToOriginalText = false)
        field.performKeyInput { pressKey(Key.Backspace) }
        ui.runOnIdle { assertEquals("**bol** tail", state.text.toString()) }
        assertEquals("bol tail", rendered())
        ui.onNodeWithText("Undo").performClick()
        assertEquals("bold tail", rendered())
        ui.onNodeWithText("Redo").performClick()
        assertEquals("bol tail", rendered())
    }

    @Test fun formatting_toolbar_and_typed_syntax_share_one_source() {
        open("hello")
        field.performTextInputSelection(TextRange(0, 5))
        ui.onNodeWithContentDescription("Bold").performClick()
        ui.runOnIdle { assertEquals("**hello**", state.text.toString()) }
        assertEquals("hello", rendered())
        ui.onNodeWithText("Undo").performClick()
        ui.runOnIdle { assertEquals("hello", state.text.toString()) }
    }

    @Test fun selection_replacement_inside_formatting_preserves_delimiters() {
        open("**bold** tail")
        field.performTextInputSelection(TextRange(1, 3), relativeToOriginalText = false)
        field.performTextInput("XY")
        ui.runOnIdle { assertEquals("**bXYd** tail", state.text.toString()) }
        assertEquals("bXYd tail", rendered())
    }

    @Test fun backspace_deletes_complete_joined_emoji() {
        open("a👩🏽‍💻")
        field.performTextInputSelection(TextRange(state.text.length))
        field.performKeyInput { pressKey(Key.Backspace) }
        ui.runOnIdle { assertEquals("a", state.text.toString()) }
    }

    @Test fun incomplete_syntax_and_multilingual_insertion_are_preserved() {
        open("**incomplete")
        assertEquals("**incomplete", rendered())
        field.performTextInputSelection(TextRange(state.text.length))
        field.performTextInput(" café שלום العربية 👩🏽‍💻**")
        assertEquals("incomplete café שלום العربية 👩🏽‍💻", rendered())
    }

    @Test fun forward_delete_at_opening_delimiter_preserves_formatting() {
        open("**bold** tail")
        field.performTextInputSelection(TextRange(0), relativeToOriginalText = false)
        field.performKeyInput { pressKey(Key.Delete) }
        ui.runOnIdle { assertEquals("**old** tail", state.text.toString()) }
    }

    @Test fun insertion_at_hidden_opening_delimiter_preserves_formatting() {
        open("**bold** tail")
        field.performTextInputSelection(TextRange(0), relativeToOriginalText = false)
        field.performTextInput("X")
        ui.runOnIdle { assertEquals("**Xbold** tail", state.text.toString()) }
        assertEquals("Xbold tail", rendered())
    }

    @Test fun insertion_at_hidden_closing_delimiter_preserves_formatting() {
        open("**bold** tail")
        field.performTextInputSelection(TextRange(4), relativeToOriginalText = false)
        field.performTextInput("X")
        ui.runOnIdle { assertEquals("**boldX** tail", state.text.toString()) }
        assertEquals("boldX tail", rendered())
    }

    @Test fun deleting_whole_formatted_span_leaves_no_orphan_markers() {
        open("**bold** tail")
        field.performTextInputSelection(TextRange(0, 4), relativeToOriginalText = false)
        field.performKeyInput { pressKey(Key.Backspace) }
        ui.runOnIdle { assertEquals(" tail", state.text.toString()) }
    }

    @Test fun copy_paste_uses_visible_text_and_undo_restores_source() {
        open("**bold** tail")
        field.performTextInputSelection(TextRange(0, 4), relativeToOriginalText = false)
        field.performKeyInput { keyDown(Key.CtrlLeft); pressKey(Key.C); keyUp(Key.CtrlLeft) }
        ui.runOnIdle {
            assertEquals("bold", (clipboard.entry!!.nativeClipEntry as Transferable).getTransferData(DataFlavor.stringFlavor))
        }
        field.performTextInputSelection(TextRange(state.text.length))
        field.performKeyInput { keyDown(Key.CtrlLeft); pressKey(Key.V); keyUp(Key.CtrlLeft) }
        ui.runOnIdle { assertEquals("**bold** tailbold", state.text.toString()) }
        ui.onNodeWithText("Undo").performClick()
        ui.runOnIdle { assertEquals("**bold** tail", state.text.toString()) }
    }

    @Test fun tab_leaves_editor_and_shift_tab_returns_without_inserting_text() {
        state = TextFieldState("**bold** tail")
        ui.setContent {
            MaterialTheme {
                Column {
                    Composer(state, NativeCore::analyze)
                    TextButton({}) { Text("Next control") }
                }
            }
        }
        field.performClick()
        field.performKeyInput { pressKey(Key.Tab) }
        ui.onNodeWithText("Next control").assertIsFocused()
            .performKeyInput { keyDown(Key.ShiftLeft); pressKey(Key.Tab); keyUp(Key.ShiftLeft) }
        field.assertIsFocused()
        ui.runOnIdle { assertEquals("**bold** tail", state.text.toString()) }
    }
}
