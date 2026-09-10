package org.sigil

import kotlin.math.abs
import kotlin.test.*

class DiceGeometryTest {
    @Test fun familiar_dice_are_closed_convex_polyhedra_with_distinct_faces() {
        for(sides in listOf(4,6,8,10,12,20)) {
            val faces=diceGeometry(sides)
            assertEquals(sides,faces.size,"d$sides")
            assertEquals((1..sides).toSet(),faces.map {it.number}.toSet())
            val all=faces.flatMap {it.vertices}.distinct()
            val edges=mutableMapOf<Set<Vertex>,Int>()
            for(face in faces) {
                assertTrue(face.normal.dot(face.center)>0)
                assertTrue(all.all {face.normal.dot(it-face.center)<.0001f})
                val forward=face.normal.faceForward(face.normal)
                assertTrue(abs(forward.x)<.0001f && abs(forward.y)<.0001f && forward.z>.999f)
                face.vertices.indices.forEach {i->val edge=setOf(face.vertices[i],face.vertices[(i+1)%face.vertices.size]);edges[edge]=(edges[edge] ?: 0)+1}
            }
            assertTrue(edges.values.all {it==2},"Every edge belongs to two faces")
            assertEquals(2,all.size-edges.size+faces.size,"Euler characteristic")
        }
        val cube=diceGeometry(6)
        cube.forEach {face->assertEquals(7,face.number+cube.single {it.normal.dot(face.normal)<-.99f}.number)}
    }
}
