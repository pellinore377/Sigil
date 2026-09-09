package org.sigil.compose

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.net.Uri
import android.os.Build
import androidx.compose.foundation.Image
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider
import java.io.ByteArrayOutputStream

internal suspend fun stageProfilePhoto(context: Context, uri: Uri?) = withContext(Dispatchers.IO) {
    var image: Bitmap? = null
    var cropped: Bitmap? = null
    var bytes = ByteArray(0)
    try {
        if (uri != null) {
            image = if (Build.VERSION.SDK_INT >= 28) ImageDecoder.decodeBitmap(ImageDecoder.createSource(context.contentResolver, uri)) { decoder, info, _ ->
                val scale = maxOf(1f, maxOf(info.size.width, info.size.height) / 512f)
                decoder.setTargetSize((info.size.width / scale).toInt().coerceAtLeast(1), (info.size.height / scale).toInt().coerceAtLeast(1))
                decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
            } else {
                val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                context.contentResolver.openInputStream(uri).use { BitmapFactory.decodeStream(it, null, options) }
                check(options.outWidth > 0 && options.outHeight > 0)
                options.inSampleSize = 1
                while (maxOf(options.outWidth, options.outHeight) / options.inSampleSize > 512) options.inSampleSize *= 2
                options.inJustDecodeBounds = false
                context.contentResolver.openInputStream(uri).use { BitmapFactory.decodeStream(it, null, options) } ?: error("Unsupported image")
            }
            val side = minOf(image.width, image.height)
            cropped = Bitmap.createBitmap(image, (image.width - side) / 2, (image.height - side) / 2, side, side)
            for (quality in listOf(88, 75, 60, 40)) {
                bytes.fill(0)
                val output = ByteArrayOutputStream()
                check(cropped.compress(Bitmap.CompressFormat.JPEG, quality, output))
                bytes = output.toByteArray()
                if (bytes.size <= 128 * 1024) break
            }
            check(bytes.size <= 128 * 1024)
        }
        check(StorageKeyProvider(context).withKey { directory, key -> NativeStorage.stageProfilePhoto(directory.path, key, bytes) })
    } finally { if (cropped !== image) cropped?.recycle(); image?.recycle(); bytes.fill(0) }
}

private val photoLoads = Semaphore(2)

@Composable
internal fun ProfilePhoto(reference: String, revision: Long, modifier: Modifier) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var image by remember(reference, revision) { mutableStateOf<Bitmap?>(null) }
    LaunchedEffect(reference, revision, lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (isActive) {
                image = try { withContext(Dispatchers.IO) { photoLoads.withPermit {
                    val bytes = StorageKeyProvider(context).withKey { directory, key -> NativeStorage.profilePhoto(directory.path, key, reference) }
                    try { bytes?.let {
                        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                        BitmapFactory.decodeByteArray(it, 0, it.size, bounds)
                        if (bounds.outWidth in 1..512 && bounds.outHeight in 1..512) BitmapFactory.decodeByteArray(it, 0, it.size) else null
                    } } finally { bytes?.fill(0) }
                } } } catch (cancelled: CancellationException) { throw cancelled } catch (_: Exception) { null }
                delay(300_000)
            }
        }
    }
    image?.let { Image(it.asImageBitmap(), null, modifier, contentScale = ContentScale.Crop) }
}
