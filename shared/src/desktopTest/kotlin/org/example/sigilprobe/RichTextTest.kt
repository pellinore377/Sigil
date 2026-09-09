package org.sigil

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.font.FontFamily
import org.junit.Test
import kotlin.test.*

class RichTextTest {
    @Test fun hidden_text_has_no_semantic_content_or_link_until_revealed() {
        val value = RichText("A secret end", listOf(RichSpan(2, 8, reveal = "spoiler", link = "https://example.com/secret")))
        var revealed = -1
        val hidden = richPresentation(value, emptySet(), FontFamily.Monospace, Color.White, Color.Black) { revealed = it }
        assertEquals("A Hidden text end", hidden.text)
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
