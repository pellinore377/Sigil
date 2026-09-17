package org.sigil

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import androidx.compose.ui.platform.testTag
import kotlin.math.roundToInt

@Composable internal fun ObjectAppearance(value:Appearance,update:(Appearance)->Unit) {
    var kind by remember {mutableIntStateOf(0)}
    val style=value.objectStyle(kind)
    fun change(next:ObjectStyle)=update(when(kind){1->value.copy(coinStyle=next);2->value.copy(cardStyle=next);else->value.copy(diceStyle=next)})
    AppearanceChoices("Objects",listOf("Dice" to "casino","Coins" to "paid","Cards" to "style"),listOf("Dice","Coins","Cards")[kind]) {kind=listOf("Dice","Coins","Cards").indexOf(it)}
    ObjectPreview(kind)
    Column(verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Text("Color source",style=MaterialTheme.typography.titleLarge)
        listOf("Personalized" to "Use your custom material colors.","Global" to "Use your app accent in every conversation.","Conversational" to "Use the chat accent, or your app accent when the chat has no theme.").forEach {(name,detail)->
            Surface(Modifier.fillMaxWidth().selectableChoice(value.objectMode==name){update(value.copy(objectMode=name))},shape=RoundedCornerShape(16.dp),color=if(value.objectMode==name)MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainer) {
                Row(Modifier.padding(horizontal=12.dp,vertical=12.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                    RadioButton(value.objectMode==name,onClick=null)
                    Column(verticalArrangement=Arrangement.spacedBy(4.dp)) {
                        Text(name,style=MaterialTheme.typography.titleMedium)
                        Text(detail,style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
        }
        if(value.objectMode=="Personalized")Text("These colors currently style objects you see. Sending your custom design to another person is not available yet.",Modifier.padding(horizontal=12.dp),style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
    }
    if(value.objectMode=="Personalized") {
        var channel by remember(kind){mutableIntStateOf(0)}
        AppearanceChoices("Material colors",listOf("Body" to "palette","Detail" to "gradient","Marks" to "text_format"),listOf("Body","Detail","Marks")[channel]) {channel=listOf("Body","Detail","Marks").indexOf(it)}
        if(channel==1 && kind==0)SettingsToggle("Follow body color","Detail shares the body color",style.followBody) {change(style.copy(followBody=it))}
        AccentPicker(when(channel){0->style.color;1->style.detailColor;else->style.ink}) {color->change(when(channel){0->style.copy(color=color);1->style.copy(second=color,followBody=false);else->style.copy(ink=color)})}
    }
    val textures=when(kind){0->listOf("Clouded","Marbled","Pearlescent","Granite");1->listOf("Smooth","Brushed","Hammered","Aged");else->listOf("Smooth","Laid paper","Linen","Parchment")}
    AppearanceChoices("Texture",textures.map {it to "texture"},textures[style.texture]) {change(style.copy(texture=textures.indexOf(it)))}
    AppearanceChoices("Surface",listOf("Polished" to "auto_awesome","Satin" to "texture","Matte" to "blur_on"),if(style.roughness<.25f)"Polished" else if(style.roughness<.55f)"Satin" else "Matte") {change(style.copy(roughness=when(it){"Polished"->.12f;"Satin"->.38f;else->.7f}))}
    if(kind==0) {
        ObjectSlider("Translucency",style.transmission,0f..1f){change(style.copy(transmission=it))}
        ObjectSlider("Resin inclusions",style.inclusions,0f..1f){change(style.copy(inclusions=it))}
    }
    if(kind==2)AppearanceChoices("Card border",listOf("Fine" to "crop_portrait","Ornate" to "filter_vintage","Geometric" to "hexagon"),listOf("Fine","Ornate","Geometric")[style.border]) {change(style.copy(border=listOf("Fine","Ornate","Geometric").indexOf(it)))}
    var advanced by remember(kind){mutableStateOf(false)}
    SigilTextButton({advanced=!advanced}) {Glyph(if(advanced)"expand_less" else "expand_more",20);Spacer(Modifier.width(8.dp));Text("Advanced")}
    Expandable(advanced) {
        Column(verticalArrangement=Arrangement.spacedBy(12.dp)) {
            ObjectSlider("Engraving depth",style.engraving,0f..1f){change(style.copy(engraving=it))}
            if(kind==0) {
                ObjectSlider("Rounded edges",style.roundness,.025f..0.28f){change(style.copy(roundness=it))}
                ObjectSlider("Resin color depth",style.absorption,0f..8f){change(style.copy(absorption=it))}
            }
        }
    }
    SigilOutlinedButton({change(defaultObjectStyle(kind))},Modifier.fillMaxWidth(),shape=RoundedCornerShape(16.dp)) {Text("Reset this material")}
}
@Composable private fun ObjectSlider(label:String,value:Float,range:ClosedFloatingPointRange<Float>,change:(Float)->Unit) {
    val percent=(((value-range.start)/(range.endInclusive-range.start))*100).roundToInt()
    Column(verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Text("$label · $percent%",style=MaterialTheme.typography.titleMedium)
        Slider(value,change,Modifier.testTag(label).semantics {contentDescription=label;stateDescription="$percent%"},valueRange=range)
    }
}
@Composable private fun ObjectPreview(kind:Int) {
    var back by remember(kind){mutableStateOf(false)}
    var sides by remember {mutableIntStateOf(6)}
    val value=remember(kind,back,sides){when(kind){
        1->RandomizerMotion("coin",frames=listOf("Heads","Tails"),selected=if(back)1 else 0,result=if(back)"Tails" else "Heads")
        2->RandomizerMotion("choice",frames=listOf("Museum","Bookstore","Café"),selected=1,result="Bookstore")
        else->when(sides){100->RandomizerMotion("dice",dice=listOf(DieFace(10,5,"tens"),DieFace(10,3,"units")),result="42");0->RandomizerMotion("dice",dice=listOf(DieFace(10,7,"tens")),result="60");else->RandomizerMotion("dice",dice=listOf(DieFace(sides,sides-1),DieFace(6,3)),result=(sides+2).toString())}
    }}
    val clock=remember(kind){TextPlayback()}
    val scene=remember(kind){MaterialTimeline()}
    val overlay=LocalMaterialOverlay.current
    Surface(Modifier.testTag("object-preview"),shape=RoundedCornerShape(24.dp),color=MaterialTheme.colorScheme.background) {
        Column {
            MessageMotion("object-preview",clock,true,randomizerDuration(value)) {
                Box(Modifier.fillMaxWidth().height(320.dp).clipToBounds().onGloballyPositioned {scene.viewport=it.boundsInWindow()}) {
                    CompositionLocalProvider(LocalMaterialTimeline provides scene.takeIf {overlay!=null},LocalCardBack provides (kind==2 && back)) {
                        Box(Modifier.align(Alignment.BottomStart).fillMaxWidth(.8f).padding(12.dp)) {RandomizerStage(value)}
                        overlay?.invoke(scene,Modifier.matchParentSize())
                    }
                }
            }
            Row(Modifier.fillMaxWidth().padding(12.dp),verticalAlignment=Alignment.CenterVertically) {
                Text("Sample result · ${value.result}",Modifier.weight(1f),style=MaterialTheme.typography.bodySmall)
                SigilTextButton({if(kind==2)back=false;clock.replay()},enabled=!LocalMotion.current.reduced && LocalAppearance.current.messageEffects) {Glyph("replay",20);Spacer(Modifier.width(8.dp));Text("Replay")}
            }
            if(kind==0)Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal=12.dp),horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                listOf(4,6,8,10,12,16,20,24,30,0,100).forEach {n->FilterChip(selected=sides==n,onClick={clock.elapsed=12000f;sides=n},label={Text(if(n==0)"d%" else "d$n")},shape=RoundedCornerShape(12.dp))}
            } else SigilTextButton({clock.elapsed=12000f;back=!back},Modifier.align(Alignment.CenterHorizontally)) {Glyph("flip",20);Spacer(Modifier.width(8.dp));Text(if(kind==1)if(back)"Show heads" else "Show tails" else if(back)"Show front" else "Show back")}
        }
    }
}
