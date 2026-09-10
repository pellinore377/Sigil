package org.sigil.compose

import android.app.job.JobScheduler
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Test

class SyncSchedulingTest {
    @Test fun delayed_maintenance_cannot_postpone_an_earlier_wakeup() {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        val context=instrumentation.targetContext
        val jobs=context.getSystemService(JobScheduler::class.java)
        try {
            instrumentation.runOnMainSync {
                jobs.cancel(22)
                jobs.cancel(23)
                NativeSync.enqueue(context,60_000)
                assertEquals(60_000L,jobs.getPendingJob(23)!!.minLatencyMillis)
                NativeSync.enqueue(context,120_000)
                assertTrue("Later maintenance replaced the earlier wake-up",jobs.getPendingJob(23)!!.minLatencyMillis<=60_000L)
                NativeSync.enqueue(context,10_000)
                assertEquals("Earlier work must advance the pending job",10_000L,jobs.getPendingJob(23)!!.minLatencyMillis)
                NativeSync.enqueue(context)
                assertEquals(0L,jobs.getPendingJob(22)!!.minLatencyMillis)
                NativeSync.enqueue(context,120_000)
                assertEquals("A push wake-up must stay immediately eligible",0L,jobs.getPendingJob(22)!!.minLatencyMillis)
                assertNotNull("Deferred work must also remain scheduled",jobs.getPendingJob(23))
                NativeSync.enable(context,false)
                assertNull(jobs.getPendingJob(22))
                assertNull(jobs.getPendingJob(23))
            }
        } finally {jobs.cancel(22);jobs.cancel(23)}
    }
}
