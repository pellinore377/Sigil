package org.sigil

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.font.FontFamily
import org.junit.Test
import kotlin.test.*

class RichTextTest {
    @Test fun code_viewers_never_extract_concealed_ranges_and_keep_utf16_offsets() {
        val value = RichText("👋\nlet x = 1;\nEnd", listOf(RichSpan(3, 13, flags = setOf("code"))), listOf(RichBlock(3, 13, "code", language = "rust")), listOf(CodeToken(3, 6, "keyword")))
        val block = visibleCodeBlocks(value).single()
        val code = richSlice(value, block.start, block.end)
        assertEquals("let x = 1;", code.text)
        assertEquals(CodeToken(0, 3, "keyword"), code.codeTokens.single())
        assertEquals(0, code.spans.single().start)
        assertEquals(10, code.spans.single().end)
        assertTrue(visibleCodeBlocks(value.copy(spans = listOf(RichSpan(0, 13, reveal = "spoiler")))).isEmpty())
    }
    @Test fun concealed_text_is_laid_out_as_written_and_its_link_stays_inert_until_revealed() {
        val value = RichText("A secret end", listOf(RichSpan(2, 8, reveal = "spoiler", link = "https://example.com/secret")))
        var revealed = -1
        val hidden = richPresentation(value, emptySet(), FontFamily.Monospace, Color.White, Color.Black) { revealed = it }
        // The veil is drawn over the real text; a tap on it uncovers rather than following the link.
        assertEquals(value.text, hidden.text)
        val link = hidden.getLinkAnnotations(0, hidden.length).single().item as LinkAnnotation.Clickable
        link.linkInteractionListener!!.onClick(link)
        assertEquals(2, revealed)
        val shown = richPresentation(value, setOf(2), FontFamily.Monospace, Color.White, Color.Black) {}
        assertEquals(value.text, shown.text)
        assertEquals("https://example.com/secret", (shown.getLinkAnnotations(0, shown.length).single().item as LinkAnnotation.Url).url)
    }
    @Test fun canonical_code_and_unicode_are_not_parsed_again() {
        val value = RichText("👩🏽‍💻 **literal**", listOf(RichSpan(8, 19, flags = setOf("code"))))
        val shown = richPresentation(value, emptySet(), FontFamily.Monospace, Color.Black, Color.White) {}
        assertEquals(value.text, shown.text)
        val style = shown.spanStyles.single()
        assertEquals(8, style.start)
        assertEquals(19, style.end)
        assertEquals(FontFamily.Monospace, style.item.fontFamily)
    }
    private fun ratio(a: Color, b: Color): Float {
        val x = a.luminance(); val y = b.luminance()
        return (maxOf(x, y) + .05f) / (minOf(x, y) + .05f)
    }
    @Test fun ink_clears_every_background_the_glyph_can_land_on() {
        val bubble = Color(0xff302b35)
        val grounds = listOf(Color(0xff14121a), Color(0xff3a3550))
        for (hue in listOf("red", "yellow", "green", "blue", "gray")) {
            val ink = textColor("${hue}2", bubble, grounds)
            for (ground in grounds + bubble) assertTrue(ratio(ink, ground) >= 4.5f, "${hue}2 on $ground")
        }
    }
    @Test fun inline_code_ink_is_corrected_against_its_own_tint_not_the_bare_bubble() {
        val value = RichText("say hello now", listOf(RichSpan(4, 9, flags = setOf("code"), colors = listOf("blue2"))))
        val shown = richPresentation(value, emptySet(), FontFamily.Monospace, Color.White, Color.Black) {}
        val ink = shown.spanStyles.first { it.start == 4 && it.item.color != Color.Unspecified }.item.color
        assertTrue(ratio(ink, lerp(Color.White, Color.Black, .08f)) >= 4.5f, "code ink $ink")
    }
    @Test fun a_gradient_paints_its_whole_ramp_however_short_the_span_is() {
        val value = RichText("Just a little gradient feeling goes a long way.", listOf(RichSpan(23, 30, colors = listOf("blue1", "purple2", "pink3"))))
        val shown = richPresentation(value, emptySet(), FontFamily.Monospace, Color.White, Color.Black) {}
        val runs = shown.spanStyles.filter { it.end - it.start == 1 && it.start >= 23 }
        assertEquals(7, runs.size)
        assertEquals(textColor("blue1", Color.White), runs.first().item.color)
        assertEquals(textColor("pink3", Color.White), runs.last().item.color)
        assertEquals(7, runs.map { it.item.color }.distinct().size)
        assertTrue(shown.spanStyles.none { it.item.brush != null })
    }
    @Test fun a_paint_never_splits_a_grapheme_cluster() {
        assertEquals(listOf(0, 7, 8, 9), graphemeCuts("\uD83D\uDC69\uD83C\uDFFD\u200D\uD83D\uDCBBab"))
        assertEquals(listOf(0, 4, 5), graphemeCuts("\uD83C\uDDEC\uD83C\uDDE7x"))
        assertEquals(listOf(0, 2, 3), graphemeCuts("e\u0301x"))
    }
    @Test fun concealed_spans_carry_no_placeholder_and_no_link_decoration() {
        val value = RichText("A secret end", listOf(RichSpan(2, 8, reveal = "scratch")))
        val shown = richPresentation(value, emptySet(), FontFamily.Monospace, Color.White, Color.Black) {}
        assertEquals(value.text, shown.text)
        assertTrue(shown.spanStyles.none { it.item.color == Color.Transparent })
        // A touch on the ink brushes a spot rather than opening it, so there is no link to decorate.
        assertTrue(shown.getLinkAnnotations(0, shown.length).isEmpty())
    }
    @Test fun every_named_shade_remains_readable_on_light_and_dark_bubbles() {
        for (surface in listOf(Color.White, Color.Black, Color(0xffe7dfea), Color(0xff302b35))) {
            for (hue in listOf("red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink", "gray")) {
                for (shade in 1..3) {
                    val a = surface.luminance(); val b = textColor("$hue$shade", surface).luminance()
                    assertTrue((maxOf(a, b) + .05f) / (minOf(a, b) + .05f) >= 4.5f, "$hue$shade on $surface")
                }
            }
        }
    }
}
