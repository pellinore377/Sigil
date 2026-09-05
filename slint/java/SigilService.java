// The reason a message arrives at all while the app is not on screen.
//
// The engine is linked into this process and holds its own Envoy socket
// (there is no push service and no server that could wake us). The moment
// Android decides the process is idle it is cached, then frozen, then killed,
// and the socket goes with it — so the notification that should have arrived
// never does. A foreground service is the platform's one sanctioned way to
// say "this process is doing something for the person right now", and its
// type on API 34+ is exactly this case: remoteMessaging.
//
// The cost is a notification the person cannot dismiss, so it is put on the
// Connection channel at IMPORTANCE_MIN: no sound, no badge, no heads-up, and
// on most shades it collapses into the silent section under one line.
//
// Declared in the manifest by the [[package.metadata.android.application.
// service]] table in Cargo.toml, not exported: only SigilNotify starts it.
//
// START_STICKY: if the system does kill the process under memory pressure it
// starts the service again, which brings the process — and with it the engine
// — back up.

package com.sigil.slint;

import android.app.Notification;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.util.Log;

public final class SigilService extends Service {
    private static final String TAG = "SigilService";

    @Override
    public IBinder onBind(Intent intent) {
        return null; // nothing binds to it; it exists to be running
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        try {
            SigilNotify.ensureChannels(this);
            Notification n = new Notification.Builder(this, SigilNotify.CH_CONNECTION)
                    .setSmallIcon(SigilNotify.smallIcon(this))
                    .setContentTitle("Sigil is connected")
                    .setOngoing(true)
                    .setShowWhen(false)
                    .setCategory(Notification.CATEGORY_SERVICE)
                    .setContentIntent(SigilNotify.openIntent(this, "", SigilNotify.SERVICE_ID))
                    .build();
            if (Build.VERSION.SDK_INT >= 34) {
                // From API 34 the type must be declared here AND in the
                // manifest, and the two must agree.
                startForeground(SigilNotify.SERVICE_ID, n,
                        ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING);
            } else {
                startForeground(SigilNotify.SERVICE_ID, n);
            }
        } catch (Throwable t) {
            // API 31+ throws when a foreground service is started from the
            // background; the app is on screen when Rust asks for this, but
            // a refusal must cost the service, not the process.
            Log.w(TAG, "startForeground: " + t);
            stopSelf();
        }
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        // Nothing to unwind: the notification goes with the service, and the
        // engine belongs to the process rather than to this.
    }
}
