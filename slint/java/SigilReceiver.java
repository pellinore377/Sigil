// The two buttons under a notification, answered without opening the app.
//
// Declared in the manifest (see the [[package.metadata.android.application.
// receiver]] table in Cargo.toml) and not exported: only our own
// PendingIntents, which carry an explicit component, ever reach it.
//
// REPLY reads what the shade typed out of the RemoteInput, hands it to the
// engine through SigilNative, and puts the line straight into the
// notification as ours — Google Messages shows the reply in the thread the
// moment the arrow is tapped, and until the engine's own timeline event comes
// back that echo is the only sign the message left the phone.
//
// MARK READ tells the engine and takes the notification away.
//
// WHEN THE NATIVES ARE NOT THERE. Android may start this process for a
// broadcast alone: no NativeActivity, no libsigil_slint.so, no engine, and
// therefore no registered natives (SigilNative's head comment says how the
// registration works). Calling one then throws UnsatisfiedLinkError, so
// `SigilNative.ready` is checked first and a reply that arrives in that state
// is dropped with a line in the log rather than lost inside an exception. The
// notification is cancelled either way, because leaving a reply field that
// silently swallows text is worse than clearing it.

package com.sigil.slint;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.util.Log;

public final class SigilReceiver extends BroadcastReceiver {
    private static final String TAG = "SigilReceiver";

    @Override
    public void onReceive(Context c, Intent intent) {
        if (c == null || intent == null) return;
        String action = intent.getAction();
        String roomId = intent.getStringExtra(SigilNotify.EXTRA_ROOM);
        if (roomId == null || roomId.isEmpty()) return;
        Context app = c.getApplicationContext();
        if (app == null) app = c;

        try {
            if (SigilNotify.ACTION_REPLY.equals(action)) {
                String text = SigilNotify.replyText(intent);
                if (text.isEmpty()) return;
                if (!SigilNative.ready) {
                    Log.w(TAG, "reply dropped: the app's natives are not registered "
                            + "(this process was started for the broadcast alone)");
                    SigilNotify.cancel(app, roomId);
                    return;
                }
                SigilNative.reply(roomId, text);
                SigilNotify.appendOwn(app, roomId, text);
                return;
            }
            if (SigilNotify.ACTION_MARK_READ.equals(action)) {
                if (SigilNative.ready) {
                    SigilNative.markRead(roomId);
                } else {
                    Log.w(TAG, "mark as read dropped: the app's natives are not registered");
                }
                SigilNotify.cancel(app, roomId);
            }
        } catch (Throwable t) {
            // A throw out of a receiver is an ANR-shaped crash on the main
            // thread of whatever process happens to be running.
            Log.w(TAG, "onReceive: " + t);
        }
    }
}
