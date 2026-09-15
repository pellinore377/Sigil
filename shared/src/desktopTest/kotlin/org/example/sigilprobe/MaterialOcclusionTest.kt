package org.sigil

import androidx.compose.ui.geometry.Rect
import kotlin.test.*
import org.junit.Test

class MaterialOcclusionTest {
    @Test fun timeline_objects_stop_at_actual_chrome_edges() {
        val bounds = MaterialOcclusion()
        bounds.header = Rect(12f, 24f, 388f, 108f)
        bounds.footer = Rect(12f, 700f, 388f, 788f)
        val visible = bounds.visible(Rect(0f, 0f, 400f, 800f))
        assertEquals(Rect(0f, 108f, 400f, 700f), visible)
        assertEquals(listOf(50f, 0f, 0f, 0f), materialClipInsets(visible, 40f, 58f, 100f, 100f))
        assertEquals(listOf(0f, 0f, 50f, 0f), materialClipInsets(visible, 40f, 650f, 100f, 100f))
        assertEquals(100f, materialClipInsets(visible, 40f, 0f, 100f, 100f)[0])
    }
@Test fun launch_corridor_reveals_only_preview_above_composer_input() {
    val bounds = MaterialOcclusion()
    bounds.header = Rect(12f, 24f, 388f, 108f)
    bounds.footer = Rect(12f, 500f, 388f, 788f)
    bounds.input = Rect(12f, 700f, 388f, 788f)
    val viewport = Rect(0f, 0f, 400f, 800f)
    assertEquals(Rect(40f, 500f, 360f, 680f), bounds.launch(viewport, Rect(40f, 530f, 360f, 680f)))
    assertEquals(Rect(40f, 108f, 360f, 700f), bounds.launch(viewport, Rect(40f, 0f, 360f, 800f)))
    assertNull(bounds.launch(viewport, null))
    bounds.input = Rect.Zero
    assertNull(bounds.launch(viewport, Rect(40f, 530f, 360f, 680f)))
}
    @Test fun offscreen_chrome_does_not_hide_timeline_and_empty_viewport_stays_hidden() {
        val bounds = MaterialOcclusion()
        bounds.header = Rect(12f, -100f, 388f, -1f)
        bounds.footer = Rect(12f, 801f, 388f, 900f)
        val viewport = Rect(0f, 0f, 400f, 800f)
        assertEquals(viewport, bounds.visible(viewport))
        assertEquals(listOf(0f, 0f, 0f, 0f), materialClipInsets(viewport, 40f, 300f, 100f, 100f))
        assertEquals(100f, materialClipInsets(Rect.Zero, 40f, 300f, 100f, 100f)[2])
    }
    @Test fun notice_is_excluded_from_both_timeline_and_overlapping_launch_corridor() {
        val visible=Rect(0f,100f,400f,600f)
        val notice=Rect(40f,120f,360f,180f)
        val regions=materialClipRegions(visible,Rect(100f,150f,300f,700f),notice)
        assertTrue(regions.isNotEmpty())
        assertTrue(regions.none {it.overlaps(notice)})
        assertTrue(regions.any {it.contains(androidx.compose.ui.geometry.Offset(200f,650f))})
        assertEquals(listOf(visible),materialClipRegions(visible,null,Rect.Zero))
        assertEquals("inset(100%)",materialClipPath(notice,null,0f,0f,notice))
    }
}
