package org.sigil

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Rule
import org.junit.Test
import kotlin.math.hypot
import kotlin.test.*

class DiagramCardTest {
    @get:Rule val ui = createComposeRule()
    private fun node(label: String, x: Float = 0f, row: Int = 0) = DiagramNode(RichText(label), "default", x, 24f + 144f * row)
    private fun edge(from: Int, to: Int, label: String = "") = DiagramEdge(from, to, RichText(label), false, 0f)
    private fun map(vararg pairs: Pair<String, String>): DiagramContent {
        val names = pairs.flatMap { listOf(it.first, it.second) }.distinct()
        return DiagramContent("mindmap", RichText(""), 0f, 0f, names.map { node(it) }, pairs.map { edge(names.indexOf(it.first), names.indexOf(it.second)) }, emptyList())
    }
    private fun Rect.distance(o: Offset) = hypot(center.x - o.x, center.y - o.y)

    private fun assertRadial(diagram: DiagramContent, width: Int, least: Int = diagram.nodes.size, size: (Int) -> IntSize) {
        val tree = diagram.mindTree()
        val rects = tree.arrange(diagram.nodes.indices.map(size), width, 12f, 4f)
        val placed = rects.withIndex().filter { it.value != null }.map { it.index to it.value!! }
        assertTrue(tree.branches.all { rects[it] != null }, "Every branch finds room")
        assertTrue(placed.size >= least, "Enough topics find room: ${placed.size} of ${diagram.nodes.size}")
        val span = placed.maxOf { it.second.right } - placed.minOf { it.second.left }
        assertTrue(span <= width + .5f, "The map fits the card: $span > $width")
        for ((a, ra) in placed) for ((b, rb) in placed) if (a < b) assertFalse(ra.overlaps(rb), "Topics $a and $b overlap")
        val root = rects[tree.root]!!.center
        for ((n, r) in placed) if (tree.depth[n] >= 2) assertTrue(r.distance(root) > rects[tree.parent[n]]!!.distance(root), "Topic $n continues outward from its branch")
    }

    @Test fun a_mind_map_radiates_from_its_central_topic_on_both_sides() {
        val studio = map("Studio" to "People", "Studio" to "Craft", "People" to "Community", "Craft" to "Details")
        val tree = studio.mindTree()
        assertEquals(0, tree.root)
        val rects = tree.arrange(List(5) { IntSize(90, 40) }, 312, 12f).map { it!! }
        val root = rects[0].center
        val (a, b) = rects[1].center - root to rects[2].center - root
        assertTrue(a.x * b.x < 0 || a.y * b.y < 0, "Two branches sit on opposite sides of the centre, not in a column beneath it")
        val wide = tree.arrange(List(5) { IntSize(90, 40) }, 900, 12f).map { it!! }
        assertTrue(wide[1].center.x > wide[0].center.x && wide[2].center.x < wide[0].center.x, "Given room, branches spread left and right")
        assertTrue(wide[3].center.x > wide[1].center.x && wide[4].center.x < wide[2].center.x, "Each branch's topic continues on its far side")
        assertRadial(studio, 312) { IntSize(90, 40) }
        assertEquals(listOf(-1, 1, 2, 1, 2), tree.family.toList(), "Each topic belongs to its first-level branch's family")
    }

    @Test fun a_larger_mind_map_keeps_every_branch_in_a_phone_bubble_and_leaves_the_rest_to_the_expanded_view() {
        val branches = listOf("Habits", "Goals", "Motivation", "Review", "Style", "Order")
        val pairs = branches.flatMap { b -> listOf("Learning" to b) + (1..2).map { b to "$b $it" } }
        assertRadial(map(*pairs.toTypedArray()), 312, least = 14) { if (it == 0) IntSize(110, 48) else IntSize(96, 40) }
        val four = listOf("Goals", "Habits", "Review", "Style").flatMap { b -> listOf("Learning" to b) + (1..2).map { b to "$b $it" } }
        assertRadial(map(*four.toTypedArray()), 312) { if (it == 0) IntSize(110, 48) else IntSize(88, 40) }
    }

    @Test fun branch_fills_keep_body_text_legible_in_all_four_bubbles() {
        val grounds = listOf(Color(0xFFE9E9E9) to Color(0xFF1B1B1B), Color(0xFF484848) to Color.White, Color(0xFF2B2B2B) to Color(0xFFE6E6E6), Color(0xFFD9D9D9) to Color(0xFF1B1B1B))
        for ((ground, ink) in grounds) for (hue in (1..6).flatMap { mindColors(it, ink, ground) }) {
            for (tint in listOf(MindBranchTint, MindLeafTint, MindSelectedTint)) assertTrue(chartContrast(ink, mindFill(ground, hue, ink, tint)) >= 4.5f, "Ink on a $tint family fill over $ground")
            assertTrue(chartContrast(mindFill(ground, hue, ink, MindBranchTint), mindFill(ground, hue, ink, MindLeafTint)) > 1.05f || chartContrast(hue, ground) < 3.2f, "A branch tile stands apart from its leaves over $ground")
            assertTrue(chartContrast(hue, ground) >= 3f, "A branch line reads against $ground")
        }
    }

    @Test fun a_state_machine_enters_at_a_dot_and_returns_along_arcs() {
        val call = DiagramContent("state", RichText(""), 0f, 0f, listOf(node("Idle", 24f, 0), node("Ringing", 24f, 1), node("Connected", 24f, 2)),
            listOf(edge(0, 1, "invite"), edge(1, 2, "answer"), edge(1, 0, "decline"), edge(2, 0, "hangup")), emptyList())
        val plan = assertNotNull(call.layerPlan(312.dp) { 56.dp })
        val chips = call.edges.indices.associateWith { IntSize(56, 24) }
        val g = assertNotNull(Density(1f).layerGeometry(call, plan, 312, call.nodes.map { IntSize(120, 44) }, chips))
        val initial = g.marks.single { !it.final }
        assertTrue(initial.center.y < g.tiles[0].top && kotlin.math.abs(initial.center.x - g.tiles[0].center.x) < 1f, "The initial dot sits above the first state")
        assertTrue(g.strokes.any { it.arrow && it.points.last().y == g.tiles[0].top }, "An arrow enters the initial state")
        assertTrue(g.strokes[2].arc && g.strokes[3].arc, "Return transitions are arcs")
        for (i in 2..3) {
            val p = g.strokes[i].points; val apex = (p[0].x + 3 * p[1].x + 3 * p[2].x + p[3].x) / 8
            assertEquals(g.chips.getValue(i).center.x, apex, 1f, "The label rides the arc's apex")
        }
        val flow = call.copy(kind = "flow")
        val f = assertNotNull(Density(1f).layerGeometry(flow, assertNotNull(flow.layerPlan(312.dp) { 56.dp }), 312, flow.nodes.map { IntSize(120, 44) }, chips))
        assertTrue(f.marks.isEmpty() && f.strokes.none { it.arc }, "A flow keeps square elbows and no state markers")
    }

    // Every map, however large, places every topic in the expanded view without overlap, quickly.
    @Test fun the_expanded_rings_hold_every_topic_of_a_large_map_quickly() {
        for ((branches, leaves, deeper) in listOf(Triple(8, 4, 0), Triple(8, 6, 0), Triple(6, 4, 1), Triple(15, 16, 0))) {
            val pairs = (1..branches).flatMap { b -> listOf("Root" to "B$b") + (1..leaves).flatMap { l -> listOf("B$b" to "B$b.$l") + (1..deeper).map { "B$b.$l" to "B$b.$l.$it" } } }
            val diagram = map(*pairs.toTypedArray()); val tree = diagram.mindTree()
            val sizes = diagram.nodes.indices.map { if (it == 0) IntSize(170, 48) else IntSize(60 + (it * 37) % 70, 40 + (it % 3) * 20) }
            val began = System.nanoTime()
            val rects = tree.radial(sizes, 12f)
            val took = (System.nanoTime() - began) / 1e6
            assertTrue(took < 50, "${diagram.nodes.size} topics laid out in ${took}ms")
            val placed = rects.map { assertNotNull(it, "Every topic is placed") }
            for (a in placed.indices) for (b in a + 1 until placed.size) assertFalse(placed[a].overlaps(placed[b]), "Topics $a and $b overlap in a ${diagram.nodes.size}-topic map")
            val root = placed[0].center
            for (n in placed.indices) if (tree.depth[n] >= 2) assertTrue(placed[n].distance(root) > placed[tree.parent[n]].distance(root), "Topic $n continues outward")
        }
    }

    @Test fun a_mind_map_reads_depth_first_with_each_topic_naming_its_parent() {
        val studio = map("Studio" to "People", "Studio" to "Craft", "People" to "Community", "Craft" to "Details")
        val tree = studio.mindTree()
        assertEquals(listOf("Studio", "People", "Community", "Craft", "Details"), tree.reading().map { studio.nodes[it].label.text })
        assertEquals("Community, under People", studio.topicDescription(tree, studio.nodes.indexOfFirst { it.label.text == "Community" }))
        assertTrue(studio.summary().startsWith("Mind map. Studio, central topic, 2 branches. Studio: People, Craft. People: Community"), studio.summary())
        ui.setContent { MaterialTheme { DiagramDetails(studio) {} } }
        val spoken = ui.onAllNodes(hasContentDescription("under", substring = true) or hasContentDescription("central topic", substring = true), useUnmergedTree = true).fetchSemanticsNodes()
        assertEquals(5, spoken.size)
        assertEquals(listOf("Studio, central topic, 2 branches", "People, under Studio", "Community, under People", "Craft, under Studio", "Details, under Craft"),
            spoken.sortedBy { it.config[SemanticsProperties.TraversalIndex] }.map { it.config[SemanticsProperties.ContentDescription].single() })
        ui.onNode(hasContentDescription("Community, under People"), useUnmergedTree = true).assert(SemanticsMatcher("Focus topic") { androidx.compose.ui.semantics.SemanticsActions.OnClick.let { k -> k in it.config && it.config[k].label == "Focus topic" } })
    }

    @Test fun every_state_with_no_way_out_ends_on_a_bullseye() {
        val order = DiagramContent("state", RichText(""), 0f, 0f, listOf(node("Placed", 136f, 0), node("Paid", 24f, 1), node("Cancelled", 248f, 1), node("Shipped", 24f, 2)),
            listOf(edge(0, 1, "pay"), edge(0, 2, "cancel"), edge(1, 3, "ship")), emptyList())
        val chips = order.edges.indices.associateWith { IntSize(48, 24) }
        val g = assertNotNull(Density(1f).layerGeometry(order, assertNotNull(order.layerPlan(312.dp) { 48.dp }), 312, order.nodes.map { IntSize(120, 44) }, chips))
        val ends = g.marks.filter { it.final }
        assertEquals(2, ends.size, "Cancelled and Shipped both end on a bullseye")
        for (n in listOf(2, 3)) assertTrue(ends.any { kotlin.math.abs(it.center.x - g.tiles[n].center.x) < 1f && it.center.y > g.tiles[n].bottom && it.center.y + 7 < (g.tiles.filter { t -> t.top > g.tiles[n].bottom }.minOfOrNull { t -> t.top } ?: g.height.toFloat()) }, "State $n's bullseye sits beneath it, clear of the next row")
        val ring = ends.first { kotlin.math.abs(it.center.x - g.tiles[2].center.x) < 1f }
        for (stroke in g.strokes.take(3)) for ((p, q) in stroke.points.zipWithNext()) if (kotlin.math.abs(p.y - q.y) < 1f) assertTrue(p.y > ring.center.y + 8 || p.y < g.tiles[2].bottom, "A bus runs beneath the bullseye, not through it")
        for (chip in g.chips.values) assertFalse(chip.overlaps(Rect(ring.center - Offset(8f, 8f), Size(16f, 16f))), "No label covers a bullseye")
    }

    @Test fun a_state_with_no_way_out_ends_on_a_bullseye() {
        val order = DiagramContent("state", RichText(""), 0f, 0f, listOf(node("Open", 24f, 0), node("Closed", 24f, 1)), listOf(edge(0, 1, "close")), emptyList())
        val g = assertNotNull(Density(1f).layerGeometry(order, assertNotNull(order.layerPlan(312.dp) { 48.dp }), 312, order.nodes.map { IntSize(120, 44) }, mapOf(0 to IntSize(48, 24))))
        val end = g.marks.single { it.final }
        assertTrue(end.center.y > g.tiles[1].bottom && end.center.y < g.height, "The bullseye hangs beneath the final state inside the card")
    }
}
