package org.sigil

import org.junit.Test
import kotlin.test.*

class ContentDecoderTest {
    @Test fun shared_decoder_preserves_native_cards_and_concealment() {
        val recipe=NativeCore.builderSource("""{"kind":"Recipe","title":"Supper","rows":[["ingredients","Rice"],["steps","Cook"]]}""")
        val decoded=ContentDecoder.part(NativeCore.structuredPreview(recipe))
        assertEquals("Supper",decoded.recipe?.title?.text)
        assertEquals("Rice",decoded.recipe?.ingredients?.single()?.text)
        assertEquals("Cook",decoded.recipe?.steps?.single()?.text)
        val table=NativeCore.builderSource("Table\nName\tCount\nRice\t2")
        assertEquals("Rice",ContentDecoder.part(NativeCore.structuredPreview(table)).table?.rows?.single()?.first()?.text)
        val chart=NativeCore.builderSource("""{"kind":"Chart","mode":"bar","title":"Test","rows":[["A","2"],["B","3"]]}""")
        assertEquals(listOf("2","3"),ContentDecoder.part(NativeCore.structuredPreview(chart)).chart?.points?.map {it.value})
        val text=ContentDecoder.part(NativeCore.structuredPreview("redact::secret; wave::Hello;"))
        assertFalse(text.text.contains("secret"))
        assertFalse(text.rich!!.text.contains("secret"))
        assertEquals("wave",text.rich!!.motion.single().kind)
    }
}
