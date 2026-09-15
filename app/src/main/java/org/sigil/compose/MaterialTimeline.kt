package org.sigil.compose

import androidx.compose.foundation.Image
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
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
    val active=LocalMotionVisible.current
    var bounds by remember {mutableStateOf(Rect.Zero)}
    val visible=active && materialViewportVisible {bounds}
    val cache=LocalImageCache.current
    val frame=MaterialFrame(kind,sides,face,font,accent,0,progress,rotation,label,true,style)
    val snapshotKey=if(progress>=1f)frame.snapshotKey() else null
    val epoch=remember(cache,snapshotKey){cache?.materialGeneration() ?: 0L}
    var snapshot by remember(cache,snapshotKey) {mutableStateOf(snapshotKey?.let {cache?.materialSnapshot(it)})}
    val capture by rememberUpdatedState<(MaterialFrame,android.graphics.Bitmap)->Unit> {rendered,image->
        if(snapshotKey!=null && rendered.sameImage(frame)) {
            cache?.rememberMaterial(snapshotKey,image,epoch)
            snapshot=image
        } else image.recycle()
    }
    var failed by remember {mutableStateOf(!MaterialNative.available)}
    if(failed)Box(modifier,contentAlignment=Alignment.Center) {Text(label ?: face.toString(),textAlign=TextAlign.Center)}
    else Box(modifier.onGloballyPositioned {bounds=it.boundsInWindow()}.clearAndSetSemantics {}) {
        if(visible) {
            val image=snapshot
            if(image!=null) Image(image.asImageBitmap(),null,Modifier.fillMaxSize(),contentScale=ContentScale.FillBounds)
            else AndroidView(factory={MessageMaterialView(it,256,if(cache!=null)({rendered,image->capture(rendered,image)})else null){failed=true}},update={it.update(frame,true)},onRelease={it.close()},modifier=Modifier.fillMaxSize())
        }
    }
}
internal object AndroidMaterials:MaterialPlatform {
    override val available get()=MaterialNative.available
    @Composable override fun Object(kind:Int,sides:Int,face:Int,rotation:FloatArray?,label:String?,modifier:Modifier,progress:Float) = MaterialObject(kind,sides,face,rotation,label,modifier,progress)
    override suspend fun record(data:FloatArray)=withContext(Dispatchers.Default){MaterialNative.record(data)}
    override fun horizontalExtent(sides:Int,face:Int,rotation:FloatArray?,outgoing:Boolean)=MaterialNative.horizontalExtent(sides,face,rotation,outgoing)
}
