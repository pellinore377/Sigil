@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class,kotlin.io.encoding.ExperimentalEncodingApi::class)
package org.sigil

import androidx.compose.runtime.*
import androidx.compose.foundation.Image
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.layout.ContentScale
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlin.io.encoding.Base64
import kotlin.js.*

private val photoLoads=Semaphore(2)
@Composable internal fun WebProfilePhoto(reference:String,revision:Int,modifier:Modifier) {
    var image by remember(reference){mutableStateOf<ImageBitmap?>(null)}
    val visible=LocalMotionVisible.current
    LaunchedEffect(reference,revision,visible) {
        if(visible)while(isActive) {
            try {
                val encoded=photoLoads.withPermit{browserProfileImage(reference).await<JsString>().toString()}
                image=if(encoded.isEmpty())null else {
                    val bytes=Base64.decode(encoded)
                    try {val decoded=org.jetbrains.skia.Image.makeFromEncoded(bytes);if(decoded.width in 1..512 && decoded.height in 1..512)decoded.toComposeImageBitmap()else null}
                    finally {bytes.fill(0)}
                }
            }catch(cancelled:CancellationException){throw cancelled}
            catch(_:Exception){}
            delay(60000)
        }
    }
    image?.let {Image(it,null,modifier,contentScale=ContentScale.Crop)}
}
