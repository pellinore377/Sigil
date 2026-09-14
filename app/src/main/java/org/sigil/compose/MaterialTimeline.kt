package org.sigil.compose

import androidx.compose.foundation.layout.Box
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.viewinterop.AndroidView
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.sigil.*

@Composable internal fun MaterialObject(kind:Int,sides:Int,face:Int,rotation:FloatArray?,label:String?,modifier:Modifier,progress:Float=1f) {
    val font=if(LocalAppearance.current.font=="Google Sans Flex")1 else 0
    val appearance=LocalAppearance.current
    val accent=(if(appearance.objectMode=="Global")LocalGlobalAccent.current else MaterialTheme.colorScheme.primary).toArgb()
    val style=remember(appearance,kind){appearance.objectStyle(kind).parameters(appearance.objectMode)}
    val visible=LocalMotionVisible.current
    var failed by remember {mutableStateOf(!MaterialNative.available)}
    if(failed)Box(modifier,contentAlignment=Alignment.Center) {Text(label ?: face.toString(),textAlign=TextAlign.Center)}
    else AndroidView(factory={MessageMaterialView(it,256){failed=true}},update={it.update(MaterialFrame(kind,sides,face,font,accent,0,progress,rotation,label,true,style),visible)},onRelease={it.close()},modifier=modifier.clearAndSetSemantics {})
}
internal object AndroidMaterials:MaterialPlatform {
    override val available get()=MaterialNative.available
    @Composable override fun Object(kind:Int,sides:Int,face:Int,rotation:FloatArray?,label:String?,modifier:Modifier,progress:Float) = MaterialObject(kind,sides,face,rotation,label,modifier,progress)
    override suspend fun record(data:FloatArray)=withContext(Dispatchers.Default){MaterialNative.record(data)}
    override fun horizontalExtent(sides:Int,face:Int,rotation:FloatArray?,outgoing:Boolean)=MaterialNative.horizontalExtent(sides,face,rotation,outgoing)
}
