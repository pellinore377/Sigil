package org.sigil.compose

import android.content.Context
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import okhttp3.*
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.ResponseBody.Companion.toResponseBody
import org.maplibre.android.MapLibre
import org.maplibre.android.camera.CameraPosition
import org.maplibre.android.geometry.LatLng
import org.maplibre.android.maps.MapView
import org.maplibre.android.maps.Style
import org.maplibre.android.module.http.HttpRequestUtil
import org.maplibre.android.offline.OfflineManager
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.util.zip.GZIPInputStream

internal object NativeMaps {
    private var configured = false
    @Synchronized fun initialize(context: Context) {
        if (configured) return
        MapLibre.getInstance(context.applicationContext)
        HttpRequestUtil.setLogEnabled(false)
        HttpRequestUtil.setPrintRequestUrlOnFailure(false)
        val dispatcher = Dispatcher().apply { maxRequests = 4; maxRequestsPerHost = 4 }
        HttpRequestUtil.setOkHttpClient(OkHttpClient.Builder().dispatcher(dispatcher).cache(null).addInterceptor { chain ->
            val request = chain.request()
            if (request.method != "GET" || request.url.scheme != "https" || request.url.host != "sigil-map.invalid" || request.url.port != 443 || request.url.query != null || request.url.fragment != null) throw IOException("Unsupported map resource")
            val path = android.net.Uri.parse(request.url.toString()).path ?: throw IOException("Invalid map resource")
            val raw = StorageKeyProvider(context).withKey { directory, key -> NativeStorage.mapResource(directory.path, key, path) } ?: throw IOException("Map resource unavailable")
            try {
                var offset = 0
                fun field(): String { val start = offset; while (offset < raw.size && raw[offset] != 10.toByte() && offset - start < 100) offset++; if (offset >= raw.size || raw[offset] != 10.toByte()) throw IOException("Invalid map response"); return raw.copyOfRange(start, offset++).toString(Charsets.US_ASCII) }
                val code = field().toInt(); val type = field(); val encoding = field()
                val body = when (encoding) {
                    "", "identity" -> raw.copyOfRange(offset, raw.size)
                    "gzip" -> GZIPInputStream(raw.inputStream(offset, raw.size - offset)).use { input ->
                        val out = ByteArrayOutputStream(); val buffer = ByteArray(8192)
                        while (true) { val count = input.read(buffer); if (count < 0) break; if (out.size() + count > 16 * 1024 * 1024) throw IOException("Map tile too large"); out.write(buffer, 0, count) }
                        out.toByteArray()
                    }
                    else -> throw IOException("Unsupported tile compression")
                }
                Response.Builder().request(request).protocol(Protocol.HTTP_1_1).code(code).message(if (code == 200) "OK" else "No Content").header("Cache-Control", "no-store").body(body.toResponseBody(type.toMediaType())).build()
            } finally { raw.fill(0) }
        }.build())
        configured = true
    }
}

@Composable
internal fun ServerMap(modifier: Modifier, latitude: Double = 0.0, longitude: Double = 0.0, movable: Boolean = true, chosen: (Double, Double) -> Unit = { _, _ -> }, failure: () -> Unit = {}) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val currentChosen by rememberUpdatedState(chosen)
    val currentFailure by rememberUpdatedState(failure)
    val view = remember { NativeMaps.initialize(context); MapView(context).apply { onCreate(null) } }
    var map by remember { mutableStateOf<org.maplibre.android.maps.MapLibreMap?>(null) }
    DisposableEffect(view) {
        var disposed = false
        OfflineManager.getInstance(context).setMaximumAmbientCacheSize(0, object : OfflineManager.FileSourceCallback {
            override fun onSuccess() { if (!disposed) view.getMapAsync { controller ->
                map = controller
                controller.uiSettings.isRotateGesturesEnabled = false
                controller.uiSettings.isScrollGesturesEnabled = movable
                controller.uiSettings.isZoomGesturesEnabled = movable
                controller.addOnMapClickListener { point -> currentChosen(point.latitude, point.longitude); true }
                controller.setStyle(Style.Builder().fromUri("https://sigil-map.invalid/client/v0/maps/style.json"))
            } }
            override fun onError(message: String) { if (!disposed) currentFailure() }
        })
        val error = MapView.OnDidFailLoadingMapListener { currentFailure() }
        view.addOnDidFailLoadingMapListener(error)
        if (lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) view.onStart()
        if (lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) view.onResume()
        val observer = LifecycleEventObserver { _, event -> when (event) { Lifecycle.Event.ON_START -> view.onStart(); Lifecycle.Event.ON_RESUME -> view.onResume(); Lifecycle.Event.ON_PAUSE -> view.onPause(); Lifecycle.Event.ON_STOP -> view.onStop(); else -> Unit } }
        lifecycle.addObserver(observer)
        onDispose { disposed = true; lifecycle.removeObserver(observer); view.removeOnDidFailLoadingMapListener(error); view.onPause(); view.onStop(); view.onDestroy() }
    }
    LaunchedEffect(map, latitude, longitude) { map?.let { controller ->
        controller.cameraPosition = CameraPosition.Builder().target(LatLng(latitude, longitude)).zoom(if (latitude == 0.0 && longitude == 0.0) 1.0 else 14.0).build()
        controller.clear()
        if (latitude != 0.0 || longitude != 0.0) controller.addMarker(org.maplibre.android.annotations.MarkerOptions().position(LatLng(latitude, longitude)))
    } }
    AndroidView({ view }, modifier)
}
