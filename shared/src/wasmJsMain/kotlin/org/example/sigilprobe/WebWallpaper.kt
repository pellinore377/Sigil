@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class,kotlin.io.encoding.ExperimentalEncodingApi::class)
package org.sigil

import androidx.compose.runtime.*
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.layout.ContentScale
import kotlinx.coroutines.*
import kotlin.io.encoding.Base64
import kotlin.js.*

@Composable internal fun WebWallpaper(peer:String,revision:Int,modifier:Modifier):Boolean {
    var image by remember(peer){mutableStateOf<ImageBitmap?>(null)}
    LaunchedEffect(peer,revision) {
        try {
            val encoded=browserWallpaperImage(peer).awaitBrowser<JsString>().toString()
            image=if(encoded.isEmpty())null else {
                val bytes=Base64.decode(encoded)
                try {val decoded=org.jetbrains.skia.Image.makeFromEncoded(bytes);if(decoded.width in 1..1600 && decoded.height in 1..1600)decoded.toComposeImageBitmap()else null}
                finally {bytes.fill(0)}
            }
        }catch(cancelled:CancellationException){throw cancelled}
        catch(_:Exception){image=null}
    }
    image?.let {Box(modifier){
        Image(it,null,Modifier.matchParentSize(),contentScale=ContentScale.Crop)
        Box(Modifier.matchParentSize().background(MaterialTheme.colorScheme.background.copy(alpha=.24f)))
    }}
    return image!=null
}
