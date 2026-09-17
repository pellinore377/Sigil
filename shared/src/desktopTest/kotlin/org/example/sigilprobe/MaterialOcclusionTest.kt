package org.sigil

import androidx.compose.ui.geometry.Rect
import kotlin.test.*
import org.junit.Test

class MaterialOcclusionTest {
    /// The chrome floats inset with rounded corners. It hides its own rectangle and
    /// nothing else: an object beside or above the pill must keep drawing, and one
    /// crossing it must not be sliced along an invisible full-width line.
    @Test fun objects_pass_behind_the_floating_chrome_instead_of_being_cut_at_its_edge() {
        val bounds = MaterialOcclusion()
        bounds.header = Rect(12f, 24f, 388f, 108f)
        bounds.footer = Rect(12f, 700f, 388f, 788f)
        val viewport = Rect(0f, 0f, 400f, 800f)
        assertEquals(viewport, bounds.visible(viewport))
        val regions = materialClipRegions(viewport, null, bounds.covered(viewport))
        val inside = { x: Float, y: Float -> regions.any { it.contains(androidx.compose.ui.geometry.Offset(x, y)) } }
        assertTrue(inside(200f, 400f), "the middle of the timeline still draws")
        assertTrue(inside(6f, 60f), "beside the header pill still draws")
        assertTrue(inside(200f, 12f), "the gap above the header pill still draws")
        assertTrue(inside(6f, 740f), "beside the footer pill still draws")
        assertTrue(inside(200f, 795f), "below the footer pill still draws")
        assertFalse(inside(200f, 60f), "behind the header pill is hidden")
        assertFalse(inside(200f, 740f), "behind the footer pill is hidden")
    }
@Test fun launch_corridor_reveals_only_preview_above_composer_input() {
    val bounds = MaterialOcclusion()
    bounds.header = Rect(12f, 24f, 388f, 108f)
    bounds.footer = Rect(12f, 500f, 388f, 788f)
    bounds.input = Rect(12f, 700f, 388f, 788f)
    val viewport = Rect(0f, 0f, 400f, 800f)
    assertEquals(Rect(40f, 500f, 360f, 680f), bounds.launch(viewport, Rect(40f, 530f, 360f, 680f)))
    assertEquals(Rect(40f, 108f, 360f, 700f), bounds.launch(viewport, Rect(40f, 0f, 360f, 800f)))
    assertTrue(materialClipRegions(viewport, bounds.launch(viewport, Rect(40f, 0f, 360f, 800f)), bounds.covered(viewport))
        .none { it.overlaps(bounds.header) }, "the corridor is still hidden behind the header")
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
        assertTrue(bounds.covered(viewport).isEmpty(), "chrome off screen covers nothing")
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
