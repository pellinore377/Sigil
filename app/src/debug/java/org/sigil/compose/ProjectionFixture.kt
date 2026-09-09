package org.sigil.compose

import android.app.*
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.projection.MediaProjectionManager
import android.os.Build
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

class ProjectionFixture : Service() {
    companion object {
        val frames = AtomicInteger()
        val failed = AtomicBoolean()
        val stopped = AtomicBoolean()
        @Volatile var failure = ""
    }
    private var screen: CallScreen? = null
    override fun onBind(intent: Intent?) = null
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        getSystemService(NotificationManager::class.java).createNotificationChannel(NotificationChannel("projection-test", "Screen capture test", NotificationManager.IMPORTANCE_LOW))
        val notification = Notification.Builder(this, "projection-test").setSmallIcon(android.R.drawable.ic_menu_view).setContentTitle("Synthetic screen test").build()
        if (Build.VERSION.SDK_INT >= 29) startForeground(31, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION) else startForeground(31, notification)
        try {
            @Suppress("DEPRECATION") val data = requireNotNull(intent?.getParcelableExtra<Intent>("result"))
            val projection = requireNotNull(getSystemService(MediaProjectionManager::class.java).getMediaProjection(Activity.RESULT_OK, data))
            screen = CallScreen(this, projection, { _, _, _ -> if (frames.incrementAndGet() >= 20) stopSelf() }, { error -> failed.set(error != null); failure = error.toString(); stopSelf() })
        } catch (error: Exception) { failure = error.toString(); failed.set(true); stopSelf() }
        return START_NOT_STICKY
    }
    override fun onDestroy() { screen?.close(); screen = null; stopped.set(true); super.onDestroy() }
}
