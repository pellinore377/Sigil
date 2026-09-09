package org.sigil.compose

import android.app.job.*
import android.content.ComponentName
import android.content.Context
import android.os.Handler
import android.os.Looper
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONObject
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider
import java.io.File

internal object NativeSync {
    private const val PERIODIC = 21
    private const val PENDING = 22
    private val sync = Mutex()
    private val transfers = Mutex()
    private val presence = Mutex()
    @Volatile private var foreground = false
    @Volatile private var interaction = 0L
    private var presenceAt = 0L
    private var presenceStatus = ""
    fun foreground(value: Boolean) { foreground = value; if (value) interaction() }
    fun interaction() { interaction = android.os.SystemClock.elapsedRealtime() }
    suspend fun presence(context: Context, inCall: Boolean) = withContext(Dispatchers.IO) {
        presence.withLock {
            if (NativeSignOut.pending(context)) return@withLock
            val now = android.os.SystemClock.elapsedRealtime()
            val status = if (inCall) "busy" else if (!foreground) "inactive" else if (now - interaction >= 300_000) "away" else "active"
            if (status == presenceStatus && now - presenceAt < 40_000) return@withLock
            val request = JSONObject().put("command", "presence").put("status", status)
            val result = try { StorageKeyProvider(context).withKey { directory, key -> JSONObject(NativeStorage.execute(directory.path, key, request.toString())) } }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { return@withLock }
            if (!result.getBoolean("ok")) return@withLock
            presenceAt = now; presenceStatus = status
        }
    }
    suspend fun files(context: Context): JSONObject = withContext(Dispatchers.IO) {
        transfers.withLock {
            check(!NativeSignOut.pending(context))
            val result = StorageKeyProvider(context).withKey { directory, key -> JSONObject(NativeStorage.execute(directory.path, key, "{\"command\":\"file_work\"}")) }
            check(result.getBoolean("ok")); result.getJSONObject("value")
        }
    }
    suspend fun run(context: Context, interactive: Boolean = false): JSONObject = withContext(Dispatchers.IO) {
        sync.withLock {
            check(!NativeSignOut.pending(context))
            val request = JSONObject().put("command", "sync").put("interactive", interactive).toString()
            val result = StorageKeyProvider(context).withKey { directory, key -> JSONObject(NativeStorage.execute(directory.path, key, request)) }
            check(result.getBoolean("ok")); result.getJSONObject("value")
        }
    }
    fun enable(context: Context, enabled: Boolean) {
        val jobs = context.getSystemService(JobScheduler::class.java)
        if (!enabled || NativeSignOut.pending(context)) { jobs.cancel(PERIODIC); jobs.cancel(PENDING); return }
        if (jobs.getPendingJob(PERIODIC) == null) jobs.schedule(base(context, PERIODIC).setPeriodic(15 * 60_000L).build())
    }
    fun enqueue(context: Context, delay: Long = 0) {
        if (NativeSignOut.pending(context)) return
        context.getSystemService(JobScheduler::class.java).schedule(base(context, PENDING).setMinimumLatency(delay.coerceAtLeast(0)).build())
    }
    private fun base(context: Context, id: Int) = JobInfo.Builder(id, ComponentName(context, SyncService::class.java)).setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY).setPersisted(true).setBackoffCriteria(10_000L, JobInfo.BACKOFF_POLICY_EXPONENTIAL)
}
class SyncService : JobService() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val main = Handler(Looper.getMainLooper())
    private val work = mutableMapOf<Int, Job>()
    override fun onStartJob(params: JobParameters): Boolean {
        work[params.jobId]?.cancel()
        work[params.jobId] = scope.launch {
            var retry = false
            try {
                if (!NativeSignOut.pending(this@SyncService) && File(noBackupFilesDir, "native/client.db").isFile) {
                    val value = NativeSync.run(this@SyncService)
                    retry = !value.isNull("issue")
                    val files = NativeSync.files(this@SyncService)
                    retry = retry || !files.isNull("issue")
                    val next = listOfNotNull(value.getLong("next_at").takeIf { !value.getBoolean("ran") || value.getBoolean("pending") || files.getInt("sent") > 0 }, files.getLong("next_at").takeIf { files.getBoolean("pending") }).minOrNull()
                    if (next != null) NativeSync.enqueue(this@SyncService, (next * 1000 - System.currentTimeMillis()).coerceAtLeast(1000))
                    NativeNotifications.update(this@SyncService)
                }
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { retry = !NativeSignOut.pending(this@SyncService) }
            val current = coroutineContext.job
            main.post { if (work[params.jobId] === current) { work.remove(params.jobId); if (!current.isCancelled) jobFinished(params, retry) } }
        }
        return true
    }
    override fun onStopJob(params: JobParameters): Boolean { work.remove(params.jobId)?.cancel(); return true }
    override fun onDestroy() { scope.cancel(); work.clear(); super.onDestroy() }
}
