package org.sigil.compose

import android.app.ActivityManager
import android.net.Uri
import android.view.KeyEvent
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Test
import java.net.ServerSocket
import java.util.concurrent.CountDownLatch
import java.util.concurrent.FutureTask
import java.util.concurrent.TimeUnit

class SignInBrowserTest {
    @Test fun redirectAndCancellationDismissTheAuthenticationTab() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        for (redirect in listOf(true, false)) {
            val paused = CountDownLatch(1)
            val served = CountDownLatch(1)
            val resumed = CountDownLatch(1)
            ServerSocket(0, 1, java.net.InetAddress.getByName("127.0.0.1")).use { server ->
                server.soTimeout = 20000
                val responder = FutureTask {
                    server.accept().use { socket ->
                        socket.soTimeout = 20000
                        val input = socket.getInputStream().bufferedReader()
                        while (!input.readLine().isNullOrEmpty()) { }
                        check(paused.await(20, TimeUnit.SECONDS))
                        // One path segment is deliberately ignored by the account callback handler.
                        val response = if (redirect) "HTTP/1.1 303 See Other\r\nLocation: sigil://oidc/browser-check\r\n" else "HTTP/1.1 200 OK\r\n"
                        val body = "<!doctype html><title>Sigil browser test</title><p>Navigation test</p>"
                        served.countDown()
                        socket.getOutputStream().write("${response}Content-Type: text/html\r\nContent-Length: ${body.length}\r\nConnection: close\r\n\r\n$body".toByteArray())
                    }
                }
                Thread(responder).start()
                ActivityScenario.launch(MainActivity::class.java).use { scenario ->
                    scenario.onActivity { activity ->
                        activity.lifecycle.addObserver(LifecycleEventObserver { _, event ->
                            if (event == Lifecycle.Event.ON_PAUSE) paused.countDown()
                            if (event == Lifecycle.Event.ON_RESUME && served.count == 0L) resumed.countDown()
                        })
                        activity.openSignIn(Uri.parse("http://127.0.0.1:${server.localPort}/"))
                    }
                    assertTrue("Browser did not load the fixture", served.await(20, TimeUnit.SECONDS))
                    if (!redirect) instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
                    assertTrue("Authentication tab did not return automatically", resumed.await(20, TimeUnit.SECONDS))
                    scenario.onActivity { activity ->
                        val tasks = activity.getSystemService(ActivityManager::class.java).appTasks
                        assertTrue(tasks.any { it.taskInfo.topActivity?.className == MainActivity::class.java.name })
                    }
                }
                responder.get(20, TimeUnit.SECONDS)
            }
        }
    }
}
