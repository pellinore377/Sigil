package org.sigil

import kotlin.math.*

internal data class Vertex(val x:Float,val y:Float,val z:Float) {
    operator fun plus(v:Vertex)=Vertex(x+v.x,y+v.y,z+v.z)
    operator fun minus(v:Vertex)=Vertex(x-v.x,y-v.y,z-v.z)
    operator fun times(n:Float)=Vertex(x*n,y*n,z*n)
    fun dot(v:Vertex)=x*v.x+y*v.y+z*v.z
    fun cross(v:Vertex)=Vertex(y*v.z-z*v.y,z*v.x-x*v.z,x*v.y-y*v.x)
    fun unit()=this*(1f/sqrt(dot(this)).coerceAtLeast(.00001f))
    fun rotate(xr:Float,yr:Float,zr:Float):Vertex {
        val a=Vertex(x,y*cos(xr)-z*sin(xr),y*sin(xr)+z*cos(xr))
        val b=Vertex(a.x*cos(yr)+a.z*sin(yr),a.y,-a.x*sin(yr)+a.z*cos(yr))
        return Vertex(b.x*cos(zr)-b.y*sin(zr),b.x*sin(zr)+b.y*cos(zr),b.z)
    }
}
internal data class DiePolygon(val vertices:List<Vertex>,val normal:Vertex,val number:Int) {
    val center=vertices.reduce(Vertex::plus)*(1f/vertices.size)
}
private fun hull(input:List<Vertex>):List<DiePolygon> {
    val radius=input.maxOf {sqrt(it.dot(it))}
    val vertices=input.map {it*(1f/radius)}
    val faces=linkedMapOf<Set<Int>,DiePolygon>()
    for(a in vertices.indices)for(b in a+1 until vertices.size)for(c in b+1 until vertices.size) {
        var n=(vertices[b]-vertices[a]).cross(vertices[c]-vertices[a])
        if(n.dot(n)<.000001f)continue
        n=n.unit()
        if(n.dot(vertices[a])<0)n=n*-1f
        val plane=n.dot(vertices[a])
        if(vertices.any {n.dot(it)-plane>.0001f})continue
        val ids=vertices.indices.filter {abs(n.dot(vertices[it])-plane)<.0001f}.toSet()
        if(ids in faces)continue
        val center=ids.map(vertices::get).reduce(Vertex::plus)*(1f/ids.size)
        val u=(vertices[ids.first()]-center).unit();val v=n.cross(u)
        val ordered=ids.sortedBy {val d=vertices[it]-center;atan2(d.dot(v),d.dot(u))}.map(vertices::get)
        faces[ids]=DiePolygon(ordered,n,faces.size+1)
    }
    return faces.values.toList()
}
private fun dual(vertices:List<Vertex>)=hull(vertices).map {it.normal*(1f/it.normal.dot(it.center))}
internal fun diceGeometry(sides:Int):List<DiePolygon> {
    val phi=((1+sqrt(5.0))/2).toFloat()
    val ico=buildList {for(a in listOf(-1f,1f))for(b in listOf(-phi,phi)) {add(Vertex(0f,a,b));add(Vertex(a,b,0f));add(Vertex(b,0f,a))}}
    val points=when(sides) {
        4->listOf(Vertex(1f,1f,1f),Vertex(1f,-1f,-1f),Vertex(-1f,1f,-1f),Vertex(-1f,-1f,1f))
        8->listOf(Vertex(1f,0f,0f),Vertex(-1f,0f,0f),Vertex(0f,1f,0f),Vertex(0f,-1f,0f),Vertex(0f,0f,1f),Vertex(0f,0f,-1f))
        10->dual(List(10) {i->val angle=(i%5*2*PI/5+(if(i<5)0.0 else PI/5)).toFloat();Vertex(cos(angle),sin(angle),if(i<5).8f else -.8f)})
        12->dual(ico)
        20->ico
        else->buildList {for(x in listOf(-1f,1f))for(y in listOf(-1f,1f))for(z in listOf(-1f,1f))add(Vertex(x,y,z))}
    }
    return hull(points).map {face->
        if(sides!=6)face else face.copy(number=when {face.normal.z>.9f->1;face.normal.z<-.9f->6;face.normal.y>.9f->2;face.normal.y<-.9f->5;face.normal.x>.9f->3;else->4})
    }
}
internal fun Vertex.faceForward(normal:Vertex):Vertex {
    val y=-atan2(normal.x,normal.z)
    val x=atan2(normal.y,sqrt(normal.x*normal.x+normal.z*normal.z))
    return rotate(0f,y,0f).rotate(x,0f,0f)
}
