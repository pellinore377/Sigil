package org.sigil.compose

import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.takeOrElse
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import java.io.ByteArrayInputStream
import org.sigil.LocalMessageSurface

@Composable
internal fun MathContent(mathml: String, expression: String, modifier: Modifier) {
    val context = LocalContext.current
    var failed by remember(mathml) { mutableStateOf(false) }
    val background = LocalMessageSurface.current.takeOrElse { MaterialTheme.colorScheme.surface }.toArgb()
    val foreground = LocalContentColor.current.toArgb()
    val supported = remember { (WebView.getCurrentWebViewPackage()?.versionName?.substringBefore('.')?.toIntOrNull() ?: 0) >= 109 }
    if (!supported || failed) { Text(expression, modifier); Text(if (failed) "Formula viewer unavailable." else "Update Android System WebView to display this formula.", style = MaterialTheme.typography.bodySmall); return }
    val css = remember(background, foreground) {
        "html,body{margin:0;background:#%06x;color:#%06x;}body{padding:16px;box-sizing:border-box;min-height:100vh;display:flex;align-items:center;}math{font-size:24px;margin:auto;}".format(background and 0xffffff, foreground and 0xffffff)
    }
    val document = remember(mathml, css) { "<!doctype html><html><head><meta name=viewport content='width=device-width, initial-scale=1'><meta http-equiv=Content-Security-Policy content=\"default-src 'none'; style-src 'unsafe-inline'\"><style>$css</style></head><body>$mathml</body></html>" }
    AndroidView(modifier = modifier, factory = {
        WebView(context).apply {
            settings.apply {
                javaScriptEnabled = false
                allowFileAccess = false
                allowContentAccess = false
                domStorageEnabled = false
                blockNetworkLoads = true
                blockNetworkImage = true
                mixedContentMode = android.webkit.WebSettings.MIXED_CONTENT_NEVER_ALLOW
                builtInZoomControls = true
                displayZoomControls = false
                setSupportZoom(true)
            }
            isSaveEnabled = false
            webViewClient = object : WebViewClient() {
                override fun onRenderProcessGone(view: WebView, detail: android.webkit.RenderProcessGoneDetail): Boolean { failed = true; return true }
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest) = true
                override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest) =
                    WebResourceResponse("text/plain", "UTF-8", ByteArrayInputStream(byteArrayOf()))
            }
        }
    }, update = { view ->
        view.setBackgroundColor(background)
        if (view.tag != document) { view.tag = document; view.loadDataWithBaseURL(null, document, "text/html", "UTF-8", null) }
    }, onRelease = { if (!failed) { it.stopLoading(); it.loadUrl("about:blank") }; it.destroy() })
}
