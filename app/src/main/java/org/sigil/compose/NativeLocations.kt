package org.sigil.compose

import android.Manifest
import android.app.*
import android.content.*
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.location.*
import android.os.*
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONObject
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider
import kotlin.math.roundToInt

internal object NativeLocations {
    private val lock = Mutex()
    private var ready = CompletableDeferred<Unit>()
    var preparing = false
        private set
    var generation = 0L
        private set
    var running = false
    private fun prefs(context: Context) = context.getSharedPreferences("location", Context.MODE_PRIVATE)
    fun stopping(context: Context) = prefs(context).getBoolean("stop_requested", false)
    fun requestStop(context: Context) { check(prefs(context).edit().putBoolean("stop_requested", true).commit()) }
    fun permitted(context: Context) = context.checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION) == PackageManager.PERMISSION_GRANTED
    fun visibleNotifications(context: Context): Boolean {
        val notifications = context.getSystemService(NotificationManager::class.java)
        return notifications.areNotificationsEnabled() && notifications.getNotificationChannel("location")?.importance != NotificationManager.IMPORTANCE_NONE
    }
    suspend fun prepare(context: Context) {
        check(permitted(context) && !NativeSignOut.pending(context) && !stopping(context))
        check(visibleNotifications(context))
        preparing = true
        generation++
        if (!running) ready = CompletableDeferred()
        try {
            context.startForegroundService(Intent(context, LocationService::class.java))
            withTimeout(5000) { ready.await() }
        } catch (error: Exception) { preparing = false; throw error }
    }
    fun prepared() { preparing = false; generation++ }
    fun started() { running = true; ready.complete(Unit) }
    suspend fun work(context: Context, point: Location? = null): JSONObject = withContext(Dispatchers.IO) {
        lock.withLock {
            check(!NativeSignOut.pending(context))
            var cursor: String? = null
            var active = 0; var queued = 0; var until = 0L; var issue: String? = null
            val stop = stopping(context)
            do {
                ensureActive()
                val command = JSONObject().put("command", "location_work").put("after", cursor).put("stop", stop)
                if (point != null && !stop) command.put("point", point.fields())
                val response = StorageKeyProvider(context).withKey { directory, key -> JSONObject(NativeStorage.execute(directory.path, key, command.toString())) }
                check(response.getBoolean("ok"))
                val value = response.getJSONObject("value")
                active += value.getInt("active"); queued += value.getInt("queued"); until = maxOf(until, value.optLong("until"))
                if (!value.isNull("issue")) issue = value.getString("issue")
                cursor = if (value.isNull("next")) null else value.getString("next")
            } while (cursor != null)
            if (queued > 0 || issue != null) NativeSync.enqueue(context)
            if (stop && issue == null) check(prefs(context).edit().remove("stop_requested").commit())
            JSONObject().put("active", active).put("until", until).put("issue", issue)
        }
    }
}

internal fun Location.fields() = JSONObject().put("coordinates", JSONObject()
    .put("latitude_e6", (latitude * 1_000_000).roundToInt()).put("longitude_e6", (longitude * 1_000_000).roundToInt()))
    .put("accuracy_cm", if (hasAccuracy()) (accuracy * 100).roundToInt().coerceAtLeast(0) else null).put("sampled_at", time / 1000)

class LocationService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var loop: Job? = null
    private var sample: Location? = null
    private var sampling = false
    private val manager by lazy { getSystemService(LocationManager::class.java) }
    private val listener = object : LocationListener {
        override fun onLocationChanged(value: Location) { sample = Location(value) }
        override fun onStatusChanged(provider: String?, status: Int, extras: Bundle?) {}
        override fun onProviderEnabled(provider: String) {}
        override fun onProviderDisabled(provider: String) { sample = null }
    }
    override fun onBind(intent: Intent?) = null
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (NativeSignOut.pending(this) || !NativeLocations.permitted(this)) { stopSampling(); stopSelf(); return START_NOT_STICKY }
        if (intent?.action == "stop") { stopSampling(); NativeLocations.requestStop(this); loop?.cancel(); loop = null }
        getSystemService(NotificationManager::class.java).createNotificationChannel(NotificationChannel("location", "Live location sharing", NotificationManager.IMPORTANCE_LOW))
        show("Preparing location sharing")
        NativeLocations.started()
        if (loop?.isActive != true) loop = scope.launch {
            var failures = 0
            while (isActive && !NativeSignOut.pending(this@LocationService)) {
                var wait = 10_000L
                try {
                    if (NativeLocations.stopping(this@LocationService)) stopSampling()
                    val fresh = sample?.takeIf { android.os.SystemClock.elapsedRealtimeNanos() - it.elapsedRealtimeNanos in 0..60_000_000_000L }
                    val generation = NativeLocations.generation
                    val value = NativeLocations.work(this@LocationService, fresh)
                    val active = value.getInt("active")
                    if (active == 0) {
                        stopSampling()
                        if (!NativeLocations.preparing && generation == NativeLocations.generation && value.isNull("issue")) break
                        show(if (NativeLocations.preparing) "Preparing location sharing" else "Sharing stopped · retrying delivery")
                    } else {
                        startSampling()
                        val end = java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT).format(java.util.Date(value.getLong("until") * 1000))
                        show(if (fresh == null) "Waiting for location · until $end" else "Sharing with $active ${if (active == 1) "conversation" else "conversations"} · until $end")
                        wait = minOf(wait, (value.getLong("until") * 1000 - System.currentTimeMillis()).coerceAtLeast(100))
                    }
                    failures = 0
                } catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) {
                    stopSampling(); failures++
                    show(if (NativeLocations.stopping(this@LocationService)) "Sharing stopped · retrying delivery" else "Location sharing paused · retrying")
                    if (!NativeLocations.permitted(this@LocationService)) { NativeLocations.requestStop(this@LocationService); NativeSync.enqueue(this@LocationService); break }
                }
                delay(if (NativeLocations.preparing) 500 else if (failures > 0) minOf(60_000L, failures * 10_000L) else wait)
            }
            stopSampling(); stopSelf()
        }
        return START_NOT_STICKY
    }
    private fun startSampling() {
        if (sampling) return
        try {
            val providers = locationProviders(this, manager)
            providers.forEach { manager.requestLocationUpdates(it, 10_000L, 0f, listener, Looper.getMainLooper()) }
            sampling = providers.isNotEmpty()
        } catch (error: Exception) { stopSampling(); throw error }
    }
    private fun stopSampling() { manager.removeUpdates(listener); sampling = false; sample = null }
    private fun show(text: String) {
        val open = PendingIntent.getActivity(this, 31, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val stop = PendingIntent.getService(this, 32, Intent(this, LocationService::class.java).setAction("stop"), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val notification = Notification.Builder(this, "location").setSmallIcon(android.R.drawable.ic_menu_mylocation)
            .setContentTitle("Sigil · Live location").setContentText(text).setContentIntent(open).setOngoing(true).setOnlyAlertOnce(true)
            .setVisibility(Notification.VISIBILITY_PRIVATE).addAction(Notification.Action.Builder(null, "Stop sharing", stop).build()).build()
        if (Build.VERSION.SDK_INT >= 29) startForeground(31, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION)
        else startForeground(31, notification)
    }
    override fun onDestroy() { scope.cancel(); stopSampling(); NativeLocations.running = false; super.onDestroy() }
}

internal fun locationProviders(context: Context, manager: LocationManager): List<String> =
    listOf(LocationManager.GPS_PROVIDER, LocationManager.NETWORK_PROVIDER).filter {
        (it != LocationManager.GPS_PROVIDER || context.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED) && manager.isProviderEnabled(it)
    }
