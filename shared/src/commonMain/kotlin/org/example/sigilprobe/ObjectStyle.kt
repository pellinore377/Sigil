package org.sigil

data class ObjectStyle(val color:Int=0x7038ba,val second:Int=0x7038ba,val ink:Int=0xf2d182,
    val roughness:Float=.19f,val transmission:Float=.85f,val absorption:Float=2.4f,
    val roundness:Float=.12f,val engraving:Float=.7f,val inclusions:Float=.3f,val border:Int=1,val texture:Int=0,val followBody:Boolean=true) {
    val detailColor:Int get()=if(followBody) ((0..2).fold(0) {rgb,i->rgb or ((((color shr (i*8))and 255)*.6f+56.1f).toInt().coerceIn(0,255) shl (i*8))}) else second
    fun parameters(mode:String)=floatArrayOf(color.toFloat(),detailColor.toFloat(),ink.toFloat(),roughness,transmission,absorption,1.49f,roundness,engraving,inclusions,border.toFloat(),when(mode){"Personalized"->0f;"Global"->1f;else->2f},texture.toFloat())
    internal fun encode()=listOf(accentText(color),accentText(second),accentText(ink),roughness,transmission,absorption,roundness,engraving,inclusions,border,texture,followBody).joinToString(",")
}
internal fun defaultObjectStyle(kind:Int)=when(kind) {
    1->ObjectStyle(0xd49e47,0xd49e47,0xb88030,followBody=false)
    2->ObjectStyle(0x302947,0x5e3875,0xe3c788,followBody=false)
    else->ObjectStyle(roundness=.055f)
}
internal fun decodeObjectStyle(text:String?,kind:Int):ObjectStyle {
    val d=defaultObjectStyle(kind);val p=text?.split(',') ?: return d
    fun number(i:Int,min:Float,max:Float,default:Float)=p.getOrNull(i)?.toFloatOrNull()?.takeIf {it.isFinite() && it in min..max} ?: default
    return ObjectStyle(p.getOrNull(0)?.let(::parseAccent) ?: d.color,p.getOrNull(1)?.let(::parseAccent) ?: d.second,p.getOrNull(2)?.let(::parseAccent) ?: d.ink,
        number(3,.045f,.9f,d.roughness),number(4,0f,1f,d.transmission),number(5,0f,8f,d.absorption),number(6,.025f,.28f,d.roundness),number(7,0f,1f,d.engraving),number(8,0f,1f,d.inclusions),p.getOrNull(9)?.toIntOrNull()?.takeIf {it in 0..2} ?: d.border,p.getOrNull(10)?.toIntOrNull()?.takeIf {it in 0..3} ?: d.texture,p.getOrNull(11)?.toBooleanStrictOrNull() ?: (kind==0 && (p.getOrNull(1)?.let(::parseAccent) in listOf(null,0x148f8a))))
}
fun Appearance.objectStyle(kind:Int)=when(kind){1->coinStyle;2->cardStyle;else->diceStyle}
