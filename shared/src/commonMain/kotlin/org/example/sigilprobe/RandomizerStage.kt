package org.sigil

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.LocalContentColor
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import kotlin.math.*

internal const val RandomizerMotionMillis=1500
private val supportedDice=setOf(4,6,8,10,12,20)
private fun coinGeometry():List<DiePolygon> {
    val a=List(32) {i->val angle=(i*2*PI/32).toFloat();Vertex(cos(angle),sin(angle),.09f)}
    val b=a.map {it.copy(z=-.09f)}
    return listOf(DiePolygon(a,Vertex(0f,0f,1f),0),DiePolygon(b.reversed(),Vertex(0f,0f,-1f),1))+
        List(32) {i->val j=(i+1)%32;DiePolygon(listOf(a[i],b[i],b[j],a[j]),Vertex(a[i].x+a[j].x,a[i].y+a[j].y,0f).unit(),-1)}
}
private fun pips(number:Int):List<Pair<Int,Int>> = when(number) {
    1->listOf(0 to 0);2->listOf(-1 to -1,1 to 1);3->listOf(-1 to -1,0 to 0,1 to 1)
    4->listOf(-1 to -1,-1 to 1,1 to -1,1 to 1)
    5->pips(4)+listOf(0 to 0)
    else->listOf(-1 to -1,-1 to 0,-1 to 1,1 to -1,1 to 0,1 to 1)
}

@Composable
internal fun RandomizerStage(value:RandomizerMotion,full:Boolean=false,rich:RichText?=null) {
    val context=LocalTextMotion.current
    val motion=LocalMotion.current
    val enabled=!full && !motion.reduced && LocalAppearance.current.messageEffects
    val visible=LocalMotionVisible.current
    LaunchedEffect(visible,context) {if(!visible)context?.clock?.elapsed=2000f}
    DisposableEffect(context) {onDispose {context?.clock?.elapsed=2000f}}
    val dice=value.dice.take(6).filter {it.sides>=2 && it.face in 1..it.sides}
    val coin=value.kind=="coin" && value.frames.size==2 && value.selected in 0..1
    val solid=value.kind=="dice" && dice.isNotEmpty() || coin
    if(!solid) {ChoiceReveal(value,rich,context,enabled);return}
    val meshes=remember(dice,coin) {if(coin)listOf(coinGeometry()) else dice.map {diceGeometry(it.sides)}}
    val measure=rememberTextMeasurer(cacheSize=128)
    val style=MaterialTheme.typography.titleMedium
    val ink=MaterialTheme.colorScheme.onPrimaryContainer
    val captionInk=LocalContentColor.current
    val surface=MaterialTheme.colorScheme.primaryContainer
    val edge=MaterialTheme.colorScheme.onSurface.copy(alpha=.28f)
    val density=androidx.compose.ui.platform.LocalDensity.current
    val description=if(coin)"Coin: ${value.result}" else if(solid)"Dice: "+dice.joinToString {"d${it.sides} · ${it.face}"} else "Chosen: ${value.result}"
    val columns=if(coin)1 else minOf(3,dice.size).coerceAtLeast(1)
    val rows=if(coin)1 else (dice.size+columns-1)/columns
    val layouts=remember(value,style,density) {
        val labels=if(coin)value.frames else meshes.flatMapIndexed {i,m->m.map {if(dice[i].sides in supportedDice)it.number.toString() else dice[i].face.toString()}}+dice.map {"d${it.sides}"}
        labels.distinct().associateWith {measure.measure(AnnotatedString(it),style,overflow=TextOverflow.Ellipsis,maxLines=1,constraints=Constraints(maxWidth=with(density) {220.dp.roundToPx()}))}
    }
    Canvas(Modifier.fillMaxWidth().height((if(coin)148 else rows*104).dp).clipToBounds().clearAndSetSemantics {contentDescription=description}) {
        val progress=if(enabled) ((context?.clock?.elapsed ?: 2000f)/RandomizerMotionMillis).coerceIn(0f,1f) else 1f
        fun project(v:Vertex,center:Offset,scale:Float):Offset {val perspective=4f/(4f-v.z);return center+Offset(v.x*scale*perspective,-v.y*scale*perspective)}
        meshes.forEachIndexed {index,mesh->
            val side=size.width/columns
            val height=size.height/rows
            val p=((progress-(if(coin)0f else index*.028f))/(1f-(if(coin)0f else index*.028f))).coerceIn(0f,1f)
            val fade=1f-p
            val hop=if(coin)sin(PI.toFloat()*p).coerceAtLeast(0f)*.45f else abs(sin(PI.toFloat()*p*2.5f))*.4f*fade
            val center=Offset(side*(index%columns+.5f)+if(coin)0f else sin(p*3*PI.toFloat()+index)*side*.07f*fade,
                height*(index/columns+.59f)-hop*height)
            val radius=minOf(side,height)*(if(coin).29f else .32f)
            drawOval(Color.Black.copy(alpha=.13f*(1f-hop)),topLeft=Offset(side*(index%columns+.5f)-radius*.82f,height*(index/columns+.88f)),size=Size(radius*1.64f,radius*.19f))
            val wanted=if(coin)value.selected else if(dice[index].sides in supportedDice)dice[index].face else mesh.first().number
            val normal=mesh.first {it.number==wanted}.normal
            val xr=(if(!coin && dice[index].sides==4).55f else .22f)+fade*(if(coin)6f else 3.2f)*PI.toFloat()
            val yr=-.25f+if(coin)sin(p*PI.toFloat())*.25f else fade*(2.4f+index*.13f)*PI.toFloat()
            val zr=if(coin)sin(p*4*PI.toFloat())*.1f*fade else fade*(1.5f+index*.09f)*PI.toFloat()
            fun transform(v:Vertex)=v.faceForward(normal).rotate(xr,yr,zr)
            val faces=mesh.map {face->face to face.vertices.map(::transform)}.sortedBy {it.second.sumOf {v->v.z.toDouble()}/it.second.size}
            faces.forEach { (face,points)->
                val normalView=transform(face.normal)
                val centerView=transform(face.center)
                if(normalView.dot(Vertex(0f,0f,4f)-centerView)<=0)return@forEach
                val path=Path().apply {points.forEachIndexed {n,v->val at=project(v,center,radius);if(n==0)moveTo(at.x,at.y)else lineTo(at.x,at.y)};close()}
                val light=(normalView.dot(Vertex(-.35f,.55f,1f).unit())*.2f).coerceIn(-.2f,.2f)
                val fill=lerp(surface,if(light>0)Color.White else Color.Black,abs(light))
                drawPath(path,fill);drawPath(path,edge,style=Stroke(.65.dp.toPx()))
                if(!coin && dice[index].sides==6) {
                    val u=(face.vertices[1]-face.vertices[0]).unit();val v=face.normal.cross(u)
                    pips(face.number).forEach {(x,y)->
                        val point=face.center+u*(x*.28f)+v*(y*.28f)
                        val pip=Path().apply {
                            repeat(16) {i->val angle=i*2*PI.toFloat()/16
                                val at=project(transform(point+u*(cos(angle)*.068f)+v*(sin(angle)*.068f)),center,radius)
                                if(i==0)moveTo(at.x,at.y) else lineTo(at.x,at.y)
                            };close()
                        }
                        clipPath(path) {drawPath(pip,ink)}
                    }
                } else {
                    val label=if(coin) {if(face.number<0)return@forEach else value.frames[face.number]}
                        else if(dice[index].sides in supportedDice)face.number.toString()
                        else if(face.number==wanted)dice[index].face.toString() else return@forEach
                    val layout=layouts[label] ?: return@forEach
                    val u=(Vertex(1f,0f,0f)-normalView*normalView.x).unit()
                    val v=normalView.cross(u)
                    val inset=points.indices.minOf {i->abs(normalView.cross(points[(i+1)%points.size]-points[i]).unit().dot(centerView-points[i]))}
                    val width=layout.size.width.coerceAtLeast(1).toFloat();val height=layout.size.height.coerceAtLeast(1).toFloat()
                    val scale=inset*1.6f/sqrt(width*width+height*height)
                    val origin=centerView-u*(width*scale/2)+v*(height*scale/2)
                    val start=project(origin,center,radius)
                    val right=(project(origin+u*(width*scale),center,radius)-start)/width
                    val down=(project(origin-v*(height*scale),center,radius)-start)/height
                    val matrix=Matrix().apply {this[0,0]=right.x;this[0,1]=right.y;this[1,0]=down.x;this[1,1]=down.y;this[3,0]=start.x;this[3,1]=start.y}
                    clipPath(path) {withTransform({transform(matrix)}) {drawText(layout,ink,topLeft=Offset.Zero)}}
                }
            }
            if(!coin)layouts["d${dice[index].sides}"]?.let {label->
                withTransform({translate(side*(index%columns+.5f),height*(index/columns+.94f));scale(.65f,.65f,Offset.Zero)}) {drawText(label,captionInk,topLeft=Offset(-label.size.width/2f,-label.size.height/2f))}
            }
        }
    }
}

@Composable
private fun ChoiceReveal(value:RandomizerMotion,rich:RichText?,context:TextMotionContext?,enabled:Boolean) {
    val moving by remember(context,enabled) {derivedStateOf {enabled && (context?.clock?.elapsed ?: 2000f)<RandomizerMotionMillis}}
    val measure=rememberTextMeasurer()
    val style=if(value.kind=="number")MaterialTheme.typography.headlineSmall else MaterialTheme.typography.bodyLarge
    val surface=MaterialTheme.colorScheme.primaryContainer
    val ink=MaterialTheme.colorScheme.onPrimaryContainer
    val density=androidx.compose.ui.platform.LocalDensity.current
    val frames=remember(value,style,density) {value.frames.take(12).map {measure.measure(AnnotatedString(it),style,maxLines=1,overflow=TextOverflow.Ellipsis,constraints=Constraints(maxWidth=with(density) {220.dp.roundToPx()}))}}
    Box(Modifier.fillMaxWidth().heightIn(min=64.dp).background(surface,RoundedCornerShape(18.dp)),contentAlignment=androidx.compose.ui.Alignment.Center) {
        CompositionLocalProvider(LocalMessageSurface provides surface) {
            RichMessageText(rich ?: RichText(value.result),Modifier.padding(12.dp).graphicsLayer {alpha=if(moving)0f else 1f},style)
        }
        if(moving && frames.isNotEmpty())Canvas(Modifier.matchParentSize()) {
            val p=((context?.clock?.elapsed ?: 2000f)/RandomizerMotionMillis).coerceIn(0f,1f)
            val travel=18f*(1f-(1f-p).pow(3))
            val frame=frames[travel.toInt()%frames.size]
            val offset=(travel%1f-.5f)*size.height*.45f
            clipRect {drawText(frame,ink,topLeft=Offset((size.width-frame.size.width)/2,(size.height-frame.size.height)/2+offset))}
        }
    }
}
