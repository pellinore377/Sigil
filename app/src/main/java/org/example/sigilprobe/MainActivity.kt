package org.sigil.compose

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.toArgb
import androidx.lifecycle.ViewModelProvider
import org.sigil.NativeCore
import org.sigil.SigilApp

class MainActivity : ComponentActivity() {
    private lateinit var messenger: Messenger
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        messenger = ViewModelProvider(this)[Messenger::class.java]
        messenger.callback(intent.data)
        intent.data = null
        val preferences = getSharedPreferences("appearance", MODE_PRIVATE)
        setContent {
            var backAvailable by remember { mutableStateOf(false) }
            var goBack by remember { mutableStateOf<() -> Unit>({}) }
            BackHandler(backAvailable) { goBack() }
            val dynamicAccent = if (Build.VERSION.SDK_INT >= 31) dynamicLightColorScheme(this).primary.toArgb() and 0xffffff else null
            LaunchedEffect(messenger.authorizationUrl) {
                messenger.authorizationUrl?.let { url ->
                    try { startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
                    catch (_: android.content.ActivityNotFoundException) { }
                    finally { messenger.browserOpened() }
                }
            }
            SigilApp(NativeCore::palette, NativeCore::analyze, messenger.state, messenger::command,
                read = { preferences.getString(it, null) }, write = { key, value -> preferences.edit().putString(key, value).apply() },
                dynamicAccent = dynamicAccent, onBackAvailable = { available, action -> backAvailable = available; goBack = action })
        }
    }
    override fun onNewIntent(intent: Intent) { super.onNewIntent(intent); messenger.callback(intent.data); intent.data = null }
    override fun onStart() { super.onStart(); messenger.foreground(true) }
    override fun onStop() { messenger.foreground(false); super.onStop() }
}
