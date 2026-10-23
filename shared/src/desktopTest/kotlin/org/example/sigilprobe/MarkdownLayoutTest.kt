package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.material3.MaterialTheme
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.compose.ui.input.key.*
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class MarkdownLayoutTest {
    @get:Rule val ui = createComposeRule()
    private fun rich(source: String) = ContentDecoder.part(NativeCore.structuredPreview(source)).rich!!

    @Test fun headings_lists_and_paragraphs_become_separate_blocks() {
        val value = rich("## A little structure\n1. Think **clearly**\n2. Send a hello\n\n`shake::this stays literal;`")
        val top = markdownTree(value)
        assertEquals(listOf("heading", "list", "paragraph"), top.map { it.block.kind })
        val list = top[1]
        assertEquals(1L, list.block.first)
        val markers = list.children.mapIndexed { index, item -> itemMarker(value, item.block, list.block.first!! + index) }
        assertEquals(listOf("1.", "2."), markers.map { it.label })
        assertTrue(value.text.startsWith("Think", markers[0].content))
    }

    @Test fun task_markers_become_state_not_text() {
        val value = rich("- [x] Sketch the idea\n- [ ] Build the prototype")
        val list = markdownTree(value).single()
        val markers = list.children.map { itemMarker(value, it.block, null) }
        assertEquals(listOf(true, false), markers.map { it.task })
        assertTrue(value.text.startsWith("Sketch", markers[0].content))
    }

    @Test fun nested_lists_hang_from_their_item_and_numbering_keeps_its_start() {
        val nested = markdownTree(rich("- outer\n  - nested")).single()
        assertEquals("list", nested.children.single().children.single().block.kind)
        val value = rich("10. ten\n11. eleven")
        val list = markdownTree(value).single()
        assertEquals(10L, list.block.first)
        assertEquals("11.", itemMarker(value, list.children[1].block, 11).label)
    }

    @Test fun motion_across_blocks_keeps_one_run_of_text() {
        val text = "One\nTwo"
        val run = TextMotion("wave", 900, "loop", 1f, 40, 0, false, "", "", listOf(0 to 3, 4 to 7))
        val value = RichText(text, blocks = listOf(RichBlock(0, 3, "paragraph"), RichBlock(4, 7, "paragraph")), motion = listOf(run))
        assertTrue(markdownTree(value).isEmpty())
        assertEquals(2, markdownTree(value.copy(motion = emptyList())).size)
        assertTrue(markdownTree(RichText("plain")).isEmpty())
    }

    @Test fun a_task_list_reads_its_state_and_hides_the_brackets() {
        val value = rich("- [x] Sketch the idea\n- [ ] Polish the details")
        ui.setContent { MaterialTheme { Box(Modifier.width(320.dp)) { RichMessageText(value) } } }
        ui.onNode(SemanticsMatcher.expectValue(SemanticsProperties.ToggleableState, ToggleableState.On)).assertExists()
        ui.onNode(SemanticsMatcher.expectValue(SemanticsProperties.ToggleableState, ToggleableState.Off)).assertExists()
        ui.onAllNodes(hasText("[x]", substring = true)).assertCountEquals(0)
        ui.onNode(hasText("Sketch the idea")).assertExists()
    }

    @Test fun small_heading_levels_are_capitals_and_quotes_keep_their_structure() {
        assertEquals("A NOTE, ÉTÉ ß", smallCapitals(RichText("A note, été ß")).text)
        val quote = rich("> First paragraph\n>\n> Second paragraph\n> - quoted item")
        ui.setContent { MaterialTheme { Box(Modifier.width(320.dp)) { RichMessageText(quote) } } }
        ui.onNode(hasText("First paragraph")).assertExists()
        ui.onNode(hasText("Second paragraph")).assertExists()
        ui.onNode(hasText("quoted item")).assertExists()
        ui.onAllNodes(hasText("•", substring = true)).assertCountEquals(0)
    }

    @Test fun wrapped_code_breaks_between_words_but_reads_as_written() {
        val (shown, map) = codeBreaks("a_b.c(d, e)::f/g")
        assertEquals("a_\u200Bb.\u200Bc(\u200Bd, e)::\u200Bf/\u200Bg", shown)
        assertEquals(shown.length, map.last())
        val value = rich("```\nlet very_long_identifier = some_module::call(a, b);\n```")
        ui.setContent { MaterialTheme { Box(Modifier.width(240.dp)) { RichMessageText(value) } } }
        ui.onNode(hasText("let very_long_identifier = some_module::call(a, b);")).assertExists()
        ui.onAllNodes(hasText("\u200B", substring = true)).assertCountEquals(0)
    }

    @Test fun headings_are_announced_as_headings() {
        val value = rich("# Field notes\n\nWe walked the long way round.")
        ui.setContent { MaterialTheme { Box(Modifier.width(320.dp)) { RichMessageText(value) } } }
        ui.onNode(hasText("Field notes") and SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading)).assertExists()
    }
}

class MentionsTest {
    @get:Rule val ui = createComposeRule()
    private val directory = listOf(MentionCandidate("@ari:studio.example", "Ari Lane", true, "aa".repeat(32)), MentionCandidate("@jules:studio.example", "Jules Hart"),
        MentionCandidate("@alex:studio.example", "Alex Moreau"), MentionCandidate("@alex:north.example", "Alex Brandt"))

    @Test fun only_a_binding_or_a_known_address_settles_who_a_mention_means() {
        val runs = mentionRuns("Hey @ari, ask @jules:studio.example or @alex. Mail a@b.example, not @nobody")
        assertEquals(listOf("ari", "jules", "alex", "nobody"), runs.map { it.handle })
        assertEquals("studio.example", runs[1].server)
        assertEquals("@alex", "Hey @ari, ask @jules:studio.example or @alex. Mail".substring(runs[2].start, runs[2].end))
        // A bare handle is a guess even when one contact answers to it: another reader's @ari may be someone else.
        assertEquals(MentionResolution.Unresolved(null, listOf(directory[0])), resolveMention(runs[0], directory))
        assertEquals(directory[1], (resolveMention(runs[1], directory) as MentionResolution.Resolved).candidate)
        assertEquals(2, (resolveMention(runs[2], directory) as MentionResolution.Unresolved).candidates.size)
        assertEquals(MentionResolution.Unresolved(null, emptyList()), resolveMention(runs[3], directory))
        assertEquals(MentionResolution.Unresolved("@kim:far.example", emptyList()), resolveMention(mentionRuns("@kim:far.example").single(), directory))
        // The sender's binding names one identity; a changed identity at the same address does not resolve.
        val bound = RichMention(4, 8, "@ari:studio.example", "aa".repeat(32))
        assertEquals(directory[0], (resolveMention(runs[0], directory, bound) as MentionResolution.Resolved).candidate)
        assertEquals(MentionResolution.Unresolved("@ari:studio.example", emptyList(), bound = true), resolveMention(runs[0], directory, bound.copy(identity = "bb".repeat(32))))
        assertEquals(MentionResolution.Unresolved("@ari:elsewhere.example", emptyList(), bound = true), resolveMention(runs[0], directory, bound.copy(address = "@ari:elsewhere.example")))
    }

    @Test fun mentions_inside_code_or_links_are_left_alone_and_bindings_attach_to_their_run() {
        val value = RichText("`@ari` and @ari", listOf(RichSpan(0, 6, flags = setOf("code"))), mentions = listOf(RichMention(11, 15, "@ari:studio.example", "aa".repeat(32))))
        val marks = mentionMarks(value, directory)
        assertEquals(listOf(11), marks.map { it.first.start })
        assertIs<MentionResolution.Resolved>(marks.single().second)
    }

    @Test fun chips_are_buttons_that_say_who_they_mean() {
        val value = RichText("Hi @alex and @ari", mentions = listOf(RichMention(13, 17, "@ari:studio.example", "aa".repeat(32))))
        ui.setContent { MaterialTheme { CompositionLocalProvider(LocalMentionDirectory provides directory) { Box(Modifier.width(360.dp)) { RichInlineText(value) } } } }
        ui.onNodeWithContentDescription("Ambiguous mention @alex, 2 people").assert(SemanticsMatcher.expectValue(SemanticsProperties.Role, androidx.compose.ui.semantics.Role.Button))
        ui.onNodeWithContentDescription("Mention, Ari Lane, verified").performClick()
        ui.onNodeWithText("@ari:studio.example").assertExists()
        ui.onNodeWithText("Copy address").assertExists()
        ui.onNodeWithText("Close").performClick()
        ui.onNodeWithContentDescription("Ambiguous mention @alex, 2 people").performClick()
        ui.onNodeWithText("Who is @alex?").assertExists()
        ui.onNodeWithText("Alex Brandt").performClick()
        ui.onNodeWithText("@alex:north.example").assertExists()
        // The copy text is the message as written: chip room is drawn, never typed.
        ui.onNode(hasText("Hi @alex and @ari")).assertExists()
    }

    @Test fun the_picker_matches_handles_and_names_not_servers() {
        assertEquals(3 to "al", mentionQuery("Hi @al", 6))
        assertNull(mentionQuery("mail a@b", 8))
        assertEquals(listOf("@alex:north.example", "@alex:studio.example"), mentionSuggestions("al", directory).map { it.address })
        assertEquals(listOf("@alex:north.example", "@alex:studio.example", "@ari:studio.example"), mentionSuggestions("a", directory).map { it.address })
        assertEquals(listOf("@jules:studio.example"), mentionSuggestions("hart", directory).map { it.address })
        assertEquals(listOf("@alex:north.example"), mentionSuggestions("alex:n", directory).map { it.address })
        assertEquals("@ari", mentionInsertion(directory[0], directory))
        assertEquals("@alex:north.example", mentionInsertion(directory[3], directory))
    }

    @Test fun picks_travel_with_the_post_in_reading_order() {
        val choices = MentionChoices()
        choices.pick("@ari", "@ari:studio.example")
        choices.pick("@alex:north.example", "@alex:north.example")
        assertEquals(listOf(mapOf("display" to "@ari", "address" to "@ari:studio.example"), mapOf("display" to "@alex:north.example", "address" to "@alex:north.example"),
            mapOf("display" to "@ari", "address" to "@ari:studio.example")), choices.take("@ari, @alex:north.example and @ari. @jules"))
        assertEquals(emptyList(), choices.take("@ari"))
    }

    @Test fun choosing_a_suggestion_replaces_the_typed_query() {
        val draft = TextFieldState("Ask @ju")
        ui.setContent { MaterialTheme { CompositionLocalProvider(LocalMentionDirectory provides directory) { Box(Modifier.width(360.dp)) { MentionPicker(draft) } } } }
        ui.onNodeWithText("Jules Hart").performClick()
        ui.waitForIdle()
        assertEquals("Ask @jules ", draft.text.toString())
    }

    @OptIn(ExperimentalTestApi::class)
    @Test fun the_keyboard_moves_through_the_picker_and_enter_chooses_instead_of_sending() {
        val draft = TextFieldState("Ping @al", TextRange(8))
        var sent = false
        ui.setContent { MaterialTheme { CompositionLocalProvider(LocalMentionDirectory provides directory) {
            val picker = remember { MentionPickerState() }
            Column(Modifier.width(360.dp)) {
                MentionPicker(draft, picker)
                BasicTextField(draft, Modifier.testTag("field").onPreviewKeyEvent { picker.onKey(it) || (it.key == Key.Enter && it.type == KeyEventType.KeyDown).also { enter -> if (enter) sent = true } })
            }
        } } }
        ui.onNodeWithTag("field").performClick()
        ui.onNodeWithTag("field").performKeyInput { pressKey(Key.DirectionDown) }
        ui.onNode(hasText("Alex Moreau", substring = true) and SemanticsMatcher.expectValue(SemanticsProperties.Selected, true)).assertExists()
        ui.onNodeWithTag("field").performKeyInput { pressKey(Key.Enter) }
        ui.waitForIdle()
        assertFalse(sent)
        assertEquals("Ping @alex:studio.example ", draft.text.toString())
        draft.edit { replace(0, length, "Ping @al"); selection = TextRange(8) }
        ui.waitForIdle()
        ui.onNodeWithTag("field").performKeyInput { pressKey(Key.Escape) }
        ui.onNodeWithContentDescription("Mention suggestions").assertDoesNotExist()
        ui.onNodeWithTag("field").performKeyInput { pressKey(Key.Enter) }
        assertTrue(sent)
    }
}

class ColorShadeTest {
    private val grounds = listOf(Color.White, Color(0xfff4f4f4), Color(0xff4a4a4a), Color(0xff484848), Color(0xff2a2a2a), Color(0xff1c1c1c), Color.Black)
    private fun contrast(a: Color, b: Color) = (maxOf(a.luminance(), b.luminance()) + .05f) / (minOf(a.luminance(), b.luminance()) + .05f)

    @Test fun brightness_steps_stay_ordered_readable_and_coloured_on_every_ground() {
        for (ground in grounds) {
            val ink = if (ground.luminance() < .18f) Color.White else Color.Black
            for (hue in listOf("red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink", "gray")) {
                val shades = (1..3).map { textColor("$hue$it", ground) }
                val l = shades.map { it.luminance() }
                assertTrue(l[0] > l[1] && l[1] > l[2], "$hue on $ground: $l")
                shades.forEach { assertTrue(contrast(it, ground) >= 4.5f, "$hue on $ground: ${contrast(it, ground)}") }
                if (hue == "gray") continue
                // Step 1 keeps its hue: chroma, and a clear distance from plain ink.
                shades.forEach { assertTrue(it.oklch().c >= .06f, "$hue on $ground: chroma ${it.oklch().c}") }
                assertTrue(deltaE(shades[0], ink) >= .10f, "$hue on $ground: ΔE ${deltaE(shades[0], ink)}")
            }
        }
    }
}
