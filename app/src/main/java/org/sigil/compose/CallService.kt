package org.sigil.compose

import android.app.*
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.*

class CallService : Service() {
    companion object { internal var owner: NativeCalls? = null }
    private var wake: PowerManager.WakeLock? = null
    override fun onBind(intent: Intent?) = null
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val calls = owner
        if (calls == null) { stopSelf(); return START_NOT_STICKY }
        if (intent?.action == "end") { calls.end(); return START_NOT_STICKY }
        getSystemService(NotificationManager::class.java).createNotificationChannel(NotificationChannel("calls", "Ongoing calls", NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(this, 1, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val end = PendingIntent.getService(this, 2, Intent(this, CallService::class.java).setAction("end"), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val notification = Notification.Builder(this, "calls").setSmallIcon(android.R.drawable.sym_action_call).setContentTitle("Sigil call").setContentText("Call in progress").setContentIntent(open).setOngoing(true).addAction(Notification.Action.Builder(null, "End", end).build()).build()
        if (Build.VERSION.SDK_INT >= 30) startForeground(7, notification, calls.foregroundTypes())
        else startForeground(7, notification)
        if (wake == null) wake = getSystemService(PowerManager::class.java).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "Sigil:call").apply { acquire(86400000) }
        calls.ready()
        return START_NOT_STICKY
    }
    override fun onTaskRemoved(rootIntent: Intent?) { if (owner != null) owner?.end() else stopSelf() }
    override fun onDestroy() { owner?.end(); wake?.let { if (it.isHeld) it.release() }; wake = null; super.onDestroy() }
}
