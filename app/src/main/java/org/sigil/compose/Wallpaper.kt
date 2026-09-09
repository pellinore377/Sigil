package org.sigil.compose

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.net.Uri
import android.os.Build
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider
import java.io.ByteArrayOutputStream

internal suspend fun saveWallpaper(context: Context, peer: String, uri: Uri?) = withContext(Dispatchers.IO) {
    var bitmap: Bitmap? = null
    var bytes = ByteArray(0)
    try {
        if (uri != null) {
            bitmap = if (Build.VERSION.SDK_INT >= 28) ImageDecoder.decodeBitmap(ImageDecoder.createSource(context.contentResolver, uri)) { decoder, info, _ ->
                val scale = maxOf(1f, maxOf(info.size.width, info.size.height) / 1600f)
                decoder.setTargetSize((info.size.width / scale).toInt().coerceAtLeast(1), (info.size.height / scale).toInt().coerceAtLeast(1))
                decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
            } else {
                val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                context.contentResolver.openInputStream(uri).use { BitmapFactory.decodeStream(it, null, options) }
                check(options.outWidth > 0 && options.outHeight > 0)
                options.inSampleSize = 1
                while (maxOf(options.outWidth, options.outHeight) / options.inSampleSize > 1600) options.inSampleSize *= 2
                options.inJustDecodeBounds = false
                context.contentResolver.openInputStream(uri).use { BitmapFactory.decodeStream(it, null, options) } ?: error("Unsupported image")
            }
            val output = ByteArrayOutputStream()
            check(bitmap.compress(Bitmap.CompressFormat.JPEG, 88, output))
            bytes = output.toByteArray()
            check(bytes.size <= 2 * 1024 * 1024)
        }
        check(StorageKeyProvider(context).withKey { directory, key -> NativeStorage.setWallpaper(directory.path, key, peer, bytes) })
    } finally { bitmap?.recycle(); bytes.fill(0) }
}

@Composable
internal fun Wallpaper(peer: String, revision: Long, modifier: Modifier) {
    val context = LocalContext.current
    val image by produceState<Bitmap?>(null, peer, revision) {
        val decoded = withContext(Dispatchers.IO) {
            val bytes = StorageKeyProvider(context).withKey { directory, key -> NativeStorage.wallpaper(directory.path, key, peer) }
            try { bytes?.let {
                val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeByteArray(it, 0, it.size, bounds)
                if (bounds.outWidth in 1..1600 && bounds.outHeight in 1..1600) BitmapFactory.decodeByteArray(it, 0, it.size) else null
            } }
            finally { bytes?.fill(0) }
        }
        value = decoded
        awaitDispose { value = null }
    }
    image?.let { bitmap -> Box(modifier) {
        Image(bitmap.asImageBitmap(), null, Modifier.matchParentSize(), contentScale = ContentScale.Crop)
        Box(Modifier.matchParentSize().background(MaterialTheme.colorScheme.background.copy(alpha = .24f)))
    } }
}
