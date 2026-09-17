package org.sigil

import androidx.compose.animation.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp

@Composable
internal fun RandomizerBuilder(enabled:Boolean,back:()->Unit,initialMode:String?=null,send:(String)->Unit) {
    var kind by rememberSaveable(initialMode) {mutableStateOf(initialMode ?: "Dice")}
    var dice by rememberSaveable {mutableStateOf(listOf("2","6"))}
    var choices by rememberSaveable {mutableStateOf(listOf(""))}
    var minimum by rememberSaveable {mutableStateOf("1")}
    var maximum by rememberSaveable {mutableStateOf("100")}
    var syntax by rememberSaveable {mutableStateOf(false)}
    val resolve=LocalBuilderSource.current
    val input=when(kind) {"Dice"->listOf(kind)+dice;"Choice"->listOf(kind)+choices.filter {it.isNotBlank()};"Number"->listOf(kind,minimum,maximum);else->listOf(kind)}.joinToString("\n")
    val source=remember(input,resolve) {resolve?.invoke(input).orEmpty()}
    val motion=LocalMotion.current
    val focus=LocalFocusManager.current
    val keyboard=LocalSoftwareKeyboardController.current
    val modes=listOf("Dice","Choice","Number","Coin")
    val action=when(kind) {"Dice"->"Roll dice";"Choice"->"Pick a choice";"Number"->"Pick a number";else->"Flip coin"}
    val sizing=rememberBuilderSizing(20.dp)
    Column(Modifier.fillMaxSize().padding(start=8.dp,end=8.dp,top=8.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        Row(sizing.measure("header"),verticalAlignment=Alignment.CenterVertically) {
            Symbol("chevron_left","Back to create",back)
            Text(when(initialMode){"Choice"->"Cards";"Number"->"Random Number";null->"Randomizer";else->initialMode},Modifier.weight(1f),style=MaterialTheme.typography.titleLarge)
            SyntaxToggle(syntax) {syntax=!syntax}
        }
        if(initialMode==null)LazyRow(sizing.measure("modes"),horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            items(modes) {mode->FilterChip(selected=kind==mode,onClick={focus.clearFocus();keyboard?.hide();kind=mode},label={Text(mode)},shape=RoundedCornerShape(12.dp))}
        }
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).wrapContentHeight(unbounded=true).then(sizing.measure("body")),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        AnimatedContent(kind,transitionSpec={
            (slideInHorizontally(motion.enter(MotionMillis)) {if(modes.indexOf(targetState)>modes.indexOf(initialState))it else -it}+fadeIn(motion.enter(MotionMillis))) togetherWith
                (slideOutHorizontally(motion.exit(MotionQuick)) {if(modes.indexOf(targetState)>modes.indexOf(initialState))-it else it}+fadeOut(motion.exit(MotionExit))) using
                SizeTransform(clip=false) {_,_->motion.tween(MotionMillis)}
        },label="Randomizer form") {mode->
            Column(verticalArrangement=Arrangement.spacedBy(12.dp)) {
                when(mode) {
                    "Dice"->{
                        Text("Set the count and sides for each group. Shapes: d4, d6, d8, d10, d12, d16, d20, d24 and d30. d100 uses a percentile pair.",style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
                        dice.chunked(2).forEachIndexed {index,group->key(index) {
                            ExpandableRow(index) {
                                NumberField(group[0],{dice=dice.toMutableList().also {v->v[index*2]=it}},"Count ${index+1}",Modifier.weight(1f))
                                NumberField(group[1],{dice=dice.toMutableList().also {v->v[index*2+1]=it}},"Sides ${index+1}",Modifier.weight(1f))
                                if(dice.size>2)SigilIconButton({dice=dice.filterIndexed {i,_->i/2!=index}}) {Glyph("close",24,"Remove dice group ${index+1}")}
                            }
                        }}
                        if(dice.size<32)SigilTextButton({dice=dice+listOf("1","6")}) {Glyph("add",20);Spacer(Modifier.width(8.dp));Text("Add dice group")}
                    }
                    "Choice"->{
                        Text("Enter at least two choices. Exact duplicates count only once.",style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
                        BuilderEntries(choices,"Choice","radio_button_unchecked") {choices=it}
                    }
                    "Number"->Row(horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                        NumberField(minimum,{minimum=it},"Minimum",Modifier.weight(1f))
                        NumberField(maximum,{maximum=it},"Maximum",Modifier.weight(1f))
                    }
                    "Coin"->Column(Modifier.fillMaxWidth(),horizontalAlignment=Alignment.CenterHorizontally,verticalArrangement=Arrangement.spacedBy(8.dp)) {
                        val side=MaterialCoinUnit*MaterialObjectScale
                        if(LocalSolidMaterial.current!=null)LocalMaterialPlatform.current.Object(1,0,0,null,"",Modifier.size(side).semantics {contentDescription="Coin"},1f)
                        else Box(Modifier.size(side).semantics {contentDescription="Coin"},contentAlignment=Alignment.Center) {Glyph("toll",48)}
                        Text("Heads or tails",style=MaterialTheme.typography.labelMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
        }
        Text(if(resolve==null)"The builder is unavailable." else if(source.isEmpty())when(kind) {"Dice"->"Use positive counts, 2–1,000,000 sides and no more than 256 dice.";"Choice"->"Add two different choices.";else->"Enter a valid inclusive range."} else "The result appears in the conversation after you send.",
            Modifier.fillMaxWidth().animateContentSize(motion.tween(MotionMillis)).semantics {liveRegion=LiveRegionMode.Polite},style=MaterialTheme.typography.bodySmall,
            color=if(source.isEmpty() && (kind!="Choice" || choices.any {it.isNotBlank()}))MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant)
        SyntaxSource(syntax,source)
        BuilderConfirm(if(LocalBuilderAction.current=="Send")action else LocalBuilderAction.current,enabled && source.isNotEmpty()) {send(source)}
        }
    }
}

@Composable
private fun NumberField(value:String,change:(String)->Unit,label:String,modifier:Modifier) {
    OutlinedTextField(value,{change(it.replace('\n',' ').replace('\r',' ').take(21))},modifier,shape=RoundedCornerShape(16.dp),singleLine=true,label={Text(label)},keyboardOptions=KeyboardOptions(keyboardType=KeyboardType.Number))
}

@Composable
private fun ExpandableRow(index:Int,content:@Composable RowScope.()->Unit) {
    val motion=LocalMotion.current
    val visible=remember {androidx.compose.animation.core.MutableTransitionState(index==0).apply {targetState=true}}
    AnimatedVisibility(visible,enter=expandVertically(motion.enter(MotionMillis),expandFrom=Alignment.Top)+slideInHorizontally(motion.enter(MotionMillis)) {it}+fadeIn(motion.enter(MotionMillis)),
        exit=shrinkVertically(motion.exit(MotionMillis),shrinkTowards=Alignment.Top)+fadeOut(motion.exit(MotionExit))) {
        Row(horizontalArrangement=Arrangement.spacedBy(10.dp),verticalAlignment=Alignment.CenterVertically,content=content)
    }
}
